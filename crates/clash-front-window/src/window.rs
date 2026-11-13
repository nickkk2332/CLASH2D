//! [`WindowFrontend`]: the [`Frontend`] that shows frames in a `winit`
//! window through `softbuffer` (12 §11, decision D-830).
//!
//! The window is as small as it can be around the parts tested without
//! one: the [`Renderer`] draws each frame into a [`PixelBuffer`], which
//! is copied to the window, and the [`InputMapper`] turns what the
//! window reports into the scene's input.
//!
//! # The application keeps the loop
//!
//! [`Frontend::poll`] pumps the window system's events without waiting
//! (`winit`'s `EventLoopExtPumpEvents`), so the application's own frame
//! loop decides when to draw and when to sleep. A frame the same as the
//! one on screen, in a window of the same size, is not drawn again
//! unless it shakes, so an idle screen costs little. The window is made on
//! the first pump, and an event loop can be made only once in a
//! program, from its main thread.
//!
//! # Size and scale
//!
//! The window is resizable. Its grid is as many whole cells as fit; when
//! that changes the frontend reports [`InputEvent::Resize`] and
//! [`Frontend::caps`] gives the new size. A cell is the theme's font
//! size in logical pixels: on a screen of scale 2 a cell of 16 is drawn
//! 32 pixels high from glyphs rasterised at that height, not magnified.
//!
//! # Leaving
//!
//! Closing the window is [`InputEvent::Close`], which asks the
//! application to quit, as Control-C does in the terminal.

use std::num::NonZeroU32;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use clash_scene::{Caps, ColorDepth, Frame, Frontend, InputEvent, Rgb};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowId};

use crate::atlas::{FontError, GlyphAtlas};
use crate::draw::{Layout, Renderer, grid_size};
use crate::input::InputMapper;
use crate::pixels::PixelBuffer;

/// How long starting waits for the window system to make the window.
const START_WAIT: Duration = Duration::from_secs(5);

/// How long each pump waits for events while starting.
const START_PUMP: Duration = Duration::from_millis(10);

/// The grid a window opens with, in cells, unless told otherwise.
pub const DEFAULT_GRID: (u16, u16) = (100, 32);

/// The smallest grid the window can be made, in cells.
const MIN_GRID: (u16, u16) = (20, 8);

/// What a window frontend is started with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowOptions {
    /// The window's title.
    pub title: String,
    /// The font file: the theme's.
    pub font: PathBuf,
    /// The height of a cell in logical pixels: the theme's font size.
    pub size: u16,
    /// The colour shown round the grid, where the window is not a whole
    /// number of cells: the theme's background.
    pub backdrop: Rgb,
    /// The grid the window opens with, in cells.
    pub grid: (u16, u16),
}

/// Why the window frontend could not start.
#[derive(Debug)]
#[non_exhaustive]
pub enum WindowError {
    /// There is no display to open a window on: on Linux and the BSDs
    /// neither `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` nor `DISPLAY` is set.
    NoDisplay,
    /// The theme's font could not be used.
    Font(FontError),
    /// The window system refused: the event loop, the window or the
    /// surface it is drawn on.
    System(String),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowError::NoDisplay => f.write_str(
                "there is no display to open a window on (neither WAYLAND_DISPLAY nor DISPLAY is set)",
            ),
            WindowError::Font(error) => error.fmt(f),
            WindowError::System(problem) => {
                write!(f, "the window system refused a window: {problem}")
            }
        }
    }
}

impl std::error::Error for WindowError {}

/// Whether a window can be opened: on Linux and the BSDs, whether a
/// Wayland or an X11 display is named in the environment; elsewhere,
/// always.
pub fn display_available() -> bool {
    display_named(|name| std::env::var_os(name))
}

/// [`display_available`] over an environment given as a lookup.
pub(crate) fn display_named(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    if cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )) {
        ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"]
            .into_iter()
            .any(|name| var(name).is_some_and(|value| !value.is_empty()))
    } else {
        true
    }
}

/// The window as a [`Frontend`]. See the [module](self) documentation.
pub struct WindowFrontend {
    event_loop: EventLoop<()>,
    shown: Shown,
}

impl std::fmt::Debug for WindowFrontend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowFrontend")
            .field("grid", &self.shown.grid)
            .field("size", &self.shown.size)
            .finish_non_exhaustive()
    }
}

/// The surface a window is drawn on.
type WindowSurface = Surface<Rc<Window>, Rc<Window>>;

/// Everything but the event loop: what the window system's events are
/// handed to.
struct Shown {
    title: String,
    /// The cell's height in logical pixels.
    size: u16,
    /// The grid asked for at the start.
    start_grid: (u16, u16),
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<WindowSurface>,
    atlas: GlyphAtlas,
    renderer: Renderer,
    picture: PixelBuffer,
    /// The frame `picture` was drawn from.
    last: Frame,
    input: InputMapper,
    events: Vec<InputEvent>,
    /// The window's inside in pixels.
    pixels: (u32, u32),
    grid: (u16, u16),
    /// How the last picture was laid out, and its zoom: what the mouse
    /// is read against.
    layout: Layout,
    zoom: f32,
    /// Why the window could not be made, while starting.
    refused: Option<String>,
    /// What went wrong showing a picture, until it is asked for.
    last_error: Option<String>,
    closed: bool,
}

impl WindowFrontend {
    /// Opens the window: loads the font, makes the event loop and the
    /// window, and waits for the window system to show it. Must be
    /// called from the program's main thread, once.
    ///
    /// The window system's libraries are loaded as the program runs,
    /// and some of them panic when a library is missing (on X11,
    /// `libxkbcommon-x11`). Such a panic while starting is caught, kept
    /// quiet, and returned as [`WindowError::System`] with its message,
    /// so that the application can fall back to the terminal.
    pub fn start(options: WindowOptions) -> Result<WindowFrontend, WindowError> {
        if !display_available() {
            return Err(WindowError::NoDisplay);
        }
        let atlas =
            GlyphAtlas::open(&options.font, u32::from(options.size)).map_err(WindowError::Font)?;
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let started =
            std::panic::catch_unwind(AssertUnwindSafe(|| WindowFrontend::open(options, atlas)));
        std::panic::set_hook(previous);
        started.unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "it failed".to_owned());
            Err(WindowError::System(message))
        })
    }

    /// The part of [`WindowFrontend::start`] that calls the window
    /// system.
    fn open(options: WindowOptions, atlas: GlyphAtlas) -> Result<WindowFrontend, WindowError> {
        let mut event_loop =
            EventLoop::new().map_err(|error| WindowError::System(error.to_string()))?;
        let mut shown = Shown {
            title: options.title,
            size: options.size,
            start_grid: options.grid,
            window: None,
            context: None,
            surface: None,
            atlas,
            renderer: Renderer::new(options.backdrop),
            picture: PixelBuffer::default(),
            last: Frame::new(0, 0),
            input: InputMapper::new(),
            events: Vec::new(),
            pixels: (0, 0),
            grid: options.grid,
            layout: Layout::default(),
            zoom: 1.0,
            refused: None,
            last_error: None,
            closed: false,
        };
        let began = Instant::now();
        while shown.surface.is_none() && shown.refused.is_none() {
            if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(START_PUMP), &mut shown)
            {
                shown.refused = Some(format!("the event loop ended with {code}"));
            }
            if began.elapsed() > START_WAIT {
                shown.refused = Some("no window appeared".to_owned());
            }
        }
        if let Some(problem) = shown.refused.take() {
            return Err(WindowError::System(problem));
        }
        Ok(WindowFrontend { event_loop, shown })
    }

    /// What went wrong showing a frame, if anything did since this was
    /// last asked.
    pub fn take_error(&mut self) -> Option<String> {
        self.shown.last_error.take()
    }
}

impl Frontend for WindowFrontend {
    fn caps(&self) -> Caps {
        Caps {
            subcell: true,
            zoom: true,
            alpha: true,
            mouse: true,
            color: ColorDepth::TrueColor,
            cols: self.shown.grid.0,
            rows: self.shown.grid.1,
        }
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        if let PumpStatus::Exit(_) = self
            .event_loop
            .pump_app_events(Some(Duration::ZERO), &mut self.shown)
        {
            self.shown.close();
        }
        core::mem::take(&mut self.shown.events)
    }

    fn present(&mut self, frame: &Frame) {
        self.shown.draw(frame);
    }
}

impl Shown {
    /// The window is closing: say so once.
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.events.push(InputEvent::Close);
        }
    }

    /// Makes the window and its surface.
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let (cell_width, cell_height) = (self.atlas.cell_width(), self.atlas.cell_height());
        let logical = |(cols, rows): (u16, u16)| {
            LogicalSize::new(u32::from(cols) * cell_width, u32::from(rows) * cell_height)
        };
        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(logical(self.start_grid))
            .with_min_inner_size(logical(MIN_GRID))
            .with_resizable(true);
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        let context = Context::new(Rc::clone(&window)).map_err(|error| error.to_string())?;
        let surface =
            Surface::new(&context, Rc::clone(&window)).map_err(|error| error.to_string())?;
        self.context = Some(context);
        self.surface = Some(surface);
        self.rescale(window.scale_factor());
        self.window = Some(Rc::clone(&window));
        self.resized(window.inner_size());
        Ok(())
    }

    /// The window moved to a screen of another scale: glyphs are made
    /// again at the height a cell now has in pixels.
    fn rescale(&mut self, scale: f64) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let height = (f64::from(self.size) * scale).round() as u32;
        if height != self.atlas.cell_height() {
            self.atlas.set_cell_height(height);
            let pixels = self.pixels;
            self.regrid(pixels);
        }
    }

    /// The window changed size.
    fn resized(&mut self, size: PhysicalSize<u32>) {
        self.pixels = (size.width, size.height);
        if let (Some(surface), Some(width), Some(height)) = (
            self.surface.as_mut(),
            NonZeroU32::new(size.width),
            NonZeroU32::new(size.height),
        ) && let Err(error) = surface.resize(width, height)
        {
            self.last_error = Some(error.to_string());
        }
        self.regrid(self.pixels);
    }

    /// Works out the grid for the window's size, and reports a change.
    fn regrid(&mut self, (width, height): (u32, u32)) {
        if width == 0 || height == 0 {
            return;
        }
        let grid = grid_size(width, height, &self.atlas);
        if grid != self.grid {
            self.grid = grid;
            self.events.push(InputEvent::Resize {
                cols: grid.0,
                rows: grid.1,
            });
        }
    }

    /// Draws a frame and shows it.
    fn draw(&mut self, frame: &Frame) {
        let (width, height) = self.pixels;
        if self.surface.is_none() || width == 0 || height == 0 {
            return;
        }
        // A frame like the one on screen, in a window of the same size
        // and scale, is not drawn again, unless it shakes.
        let unchanged = *frame == self.last
            && (width, height) == (self.picture.width(), self.picture.height())
            && self.atlas.cell_height() == self.layout.cell_height
            && !(frame.fx.shake.is_finite() && frame.fx.shake > 0.0);
        if unchanged {
            return;
        }
        self.last.clone_from(frame);
        self.picture.resize(width, height);
        self.renderer
            .draw(frame, &mut self.atlas, &mut self.picture);
        self.layout = Layout::new(frame.cols, frame.rows, &self.atlas, width, height);
        self.zoom = frame.fx.zoom;
        // A cell may have come under the pointer without it moving.
        if let Some(event) = self.input.relaid(&self.layout, self.zoom) {
            self.events.push(event);
        }
        self.show();
    }

    /// Copies the last picture drawn to the window.
    fn show(&mut self) {
        let (Some(surface), Some(window)) = (self.surface.as_mut(), self.window.as_ref()) else {
            return;
        };
        let picture = &self.picture;
        let shown = (|| {
            let mut buffer = surface.buffer_mut()?;
            if buffer.len() == picture.pixels().len() {
                buffer.copy_from_slice(picture.pixels());
            } else {
                // The window changed size since the picture was drawn:
                // what fits now is shown at the next frame.
                return Ok(());
            }
            window.pre_present_notify();
            buffer.present()
        })();
        if let Err(error) = shown {
            self.last_error = Some(error.to_string());
        }
    }
}

impl ApplicationHandler for Shown {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && self.refused.is_none()
            && let Err(problem) = self.open(event_loop)
        {
            self.refused = Some(problem);
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let event = match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.close();
                None
            }
            WindowEvent::Resized(size) => {
                self.resized(size);
                None
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.rescale(scale_factor);
                None
            }
            WindowEvent::RedrawRequested => {
                self.show();
                None
            }
            WindowEvent::Focused(focused) => {
                self.input.focus(focused);
                None
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.input.modifiers(modifiers.state());
                None
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => self.input.key(&event.logical_key, event.state),
            WindowEvent::Ime(Ime::Commit(text)) => self.input.text(&text),
            WindowEvent::CursorMoved { position, .. } => {
                self.input
                    .moved(position.x, position.y, &self.layout, self.zoom)
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.left();
                None
            }
            WindowEvent::MouseInput { state, button, .. } => self.input.button(state, button),
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.wheel(delta, f64::from(self.atlas.cell_height()))
            }
            _ => None,
        };
        self.events.extend(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_display_is_one_named_in_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        let on_unix = cfg!(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        ));
        assert!(display_named(env(&[("DISPLAY", ":0")])));
        assert!(display_named(env(&[("WAYLAND_DISPLAY", "wayland-0")])));
        assert_eq!(display_named(env(&[])), !on_unix);
        assert_eq!(display_named(env(&[("DISPLAY", "")])), !on_unix);
    }

    #[test]
    fn without_a_display_or_a_font_the_window_does_not_start() {
        let options = WindowOptions {
            title: "test".to_owned(),
            font: PathBuf::from("no/such/font.ttf"),
            size: 16,
            backdrop: Rgb::BLACK,
            grid: DEFAULT_GRID,
        };
        // Making an event loop here would need the main thread, and a
        // display; only what is refused before that is tried.
        if display_available() {
            return;
        }
        let error = WindowFrontend::start(options).unwrap_err();
        assert!(matches!(error, WindowError::NoDisplay));
        assert!(error.to_string().contains("no display"), "{error}");
        let font = WindowError::Font(FontError {
            path: PathBuf::from("mods/x/assets/font.ttf"),
            reason: "missing".to_owned(),
        });
        assert_eq!(
            font.to_string(),
            "the font `mods/x/assets/font.ttf` cannot be used: missing"
        );
    }
}
