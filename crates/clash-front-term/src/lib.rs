//! The CLASH2D terminal frontend: draws the scene's frames of glyph cells
//! in a terminal and turns key presses into input for the scene. It
//! ignores what a terminal cannot draw. This crate is outside the
//! simulation and may read the wall clock. Specified in
//! `docs/12-presentation-and-ui.md` section 11.
//!
//! # Parts
//!
//! | Part | Does |
//! |------|------|
//! | [`Renderer`] | Turns a frame into the escape sequences that show it, writing only the cells that changed since the last frame. Writes to anything, so it is tested without a terminal. |
//! | [`translate`] | Turns a `crossterm` event into an [`InputEvent`]. |
//! | [`color_depth`] | Works out how many colours the terminal has from its environment. |
//! | [`TermFrontend`] | The [`Frontend`]: owns the terminal while it lives and always gives it back. |
//!
//! # What a terminal shows
//!
//! The grid, with each cell's colours mapped to what the terminal has:
//! the exact colour with 24-bit colour, the nearest of the 256-colour
//! palette, or the cell's sixteen-colour `fallback`. Sprites are drawn in
//! the nearest cell; their scale and opacity, and the frame's shake,
//! flash, zoom, bars and tint, are ignored (12 §4.2).
//!
//! # Giving the terminal back
//!
//! [`TermFrontend::start`] puts the terminal in raw mode on its alternate
//! screen and hides the cursor. Dropping the frontend undoes all of it,
//! and so does a panic anywhere in the program: a panic hook restores the
//! terminal before the panic message is printed, so the message is
//! readable and the shell is usable afterwards.

use std::io::{self, IsTerminal, Write};
use std::sync::Once;
use std::time::Duration;

use clash_scene::{
    Ansi16, Attrs, Caps, Color, ColorDepth, Frame, FrameCell, Frontend, InputEvent, KeyChord,
    KeyCode, Mods, MouseButton, MouseEvent, MouseKind, ansi256,
};
use crossterm::event::{self, Event, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::{cursor, execute, terminal};

/// Works out how many colours a terminal has from the values of the
/// environment variables `COLORTERM` and `TERM`.
///
/// `COLORTERM` of `truecolor` or `24bit` means 24-bit colour. Otherwise a
/// `TERM` that mentions `256color` means the 256-colour palette, and
/// anything else the sixteen colours every terminal has.
pub fn color_depth(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
    match colorterm {
        Some("truecolor" | "24bit") => ColorDepth::TrueColor,
        _ if term.is_some_and(|term| term.contains("256color")) => ColorDepth::Ansi256,
        _ => ColorDepth::Ansi16,
    }
}

/// Writes the part of an SGR sequence that sets one colour.
fn write_color(out: &mut Vec<u8>, color: Color, depth: ColorDepth, background: bool) {
    match depth {
        ColorDepth::TrueColor => {
            let lead = if background { 48 } else { 38 };
            let _ = write!(
                out,
                ";{lead};2;{};{};{}",
                color.rgb.r, color.rgb.g, color.rgb.b
            );
        }
        ColorDepth::Ansi256 => {
            let lead = if background { 48 } else { 38 };
            let _ = write!(out, ";{lead};5;{}", ansi256(color.rgb));
        }
        ColorDepth::Ansi16 => {
            let _ = write!(out, ";{}", ansi16_code(color.fallback, background));
        }
    }
}

/// The SGR number of one of the sixteen colours.
fn ansi16_code(color: Ansi16, background: bool) -> u8 {
    let index = color.index();
    let base = match (index < 8, background) {
        (true, false) => 30,
        (true, true) => 40,
        (false, false) => 90 - 8,
        (false, true) => 100 - 8,
    };
    base + index
}

/// Turns frames into terminal escape sequences, writing only what changed
/// since the frame before.
///
/// ```
/// use clash_front_term::Renderer;
/// use clash_scene::{Canvas, CellStyle, ColorDepth, Frame};
///
/// let mut renderer = Renderer::new(ColorDepth::Ansi16);
/// let mut frame = Frame::new(3, 1);
/// let mut out = Vec::new();
/// renderer.render(&frame, &mut out).unwrap();
///
/// Canvas::new(&mut frame).put(1, 0, '@', CellStyle::default());
/// out.clear();
/// renderer.render(&frame, &mut out).unwrap();
/// assert_eq!(String::from_utf8(out).unwrap(), "\x1b[1;2H\x1b[0;37;40m@\x1b[0m");
/// ```
#[derive(Debug, Clone)]
pub struct Renderer {
    depth: ColorDepth,
    /// The frame on screen, sprites already drawn into it.
    shown: Frame,
    /// The frame being drawn, kept to reuse its storage.
    next: Frame,
    /// Whether `shown` is what the terminal shows.
    valid: bool,
    buffer: Vec<u8>,
}

impl Renderer {
    /// A renderer for a terminal with this many colours. Its first frame
    /// is written in full.
    pub fn new(depth: ColorDepth) -> Renderer {
        Renderer {
            depth,
            shown: Frame::new(0, 0),
            next: Frame::new(0, 0),
            valid: false,
            buffer: Vec::new(),
        }
    }

    /// How many colours the renderer maps to.
    pub fn depth(&self) -> ColorDepth {
        self.depth
    }

    /// Forgets what is on screen, so that the next frame is written in
    /// full. For after something else has drawn on the terminal.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Writes what must change on the terminal for it to show `frame`,
    /// then flushes `out`. A frame of a different size from the last, and
    /// the first frame, are written in full after clearing the screen.
    pub fn render(&mut self, frame: &Frame, out: &mut impl Write) -> io::Result<()> {
        self.next.clone_from(frame);
        self.next.snap_sprites();

        let full = !self.valid || (self.shown.cols, self.shown.rows) != (frame.cols, frame.rows);
        let buffer = &mut self.buffer;
        buffer.clear();
        if full {
            buffer.extend_from_slice(b"\x1b[0m\x1b[2J");
        }
        let cols = usize::from(frame.cols);
        // Where the terminal's cursor is, and the style it is writing in.
        let mut cursor: Option<(usize, usize)> = None;
        let mut style: Option<(Color, Color, Attrs)> = None;
        for (index, cell) in self.next.cells.iter().enumerate() {
            if !full && self.shown.cells.get(index) == Some(cell) {
                continue;
            }
            let (row, col) = (index / cols.max(1), index % cols.max(1));
            if cursor != Some((row, col)) {
                let _ = write!(buffer, "\x1b[{};{}H", row + 1, col + 1);
            }
            if style != Some((cell.fg, cell.bg, cell.attrs)) {
                write_style(buffer, cell, self.depth);
                style = Some((cell.fg, cell.bg, cell.attrs));
            }
            let mut utf8 = [0u8; 4];
            let ch = if cell.ch.is_control() { ' ' } else { cell.ch };
            buffer.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
            // Writing in the last column leaves the cursor in a state
            // that differs between terminals, so always move after it.
            cursor = (col + 1 < cols).then_some((row, col + 1));
        }
        if style.is_some() {
            buffer.extend_from_slice(b"\x1b[0m");
        }
        core::mem::swap(&mut self.shown, &mut self.next);
        self.valid = true;
        out.write_all(&self.buffer)?;
        out.flush()
    }
}

/// Writes the SGR sequence that sets a cell's whole style.
fn write_style(out: &mut Vec<u8>, cell: &FrameCell, depth: ColorDepth) {
    out.extend_from_slice(b"\x1b[0");
    for (attr, code) in [
        (Attrs::BOLD, 1),
        (Attrs::DIM, 2),
        (Attrs::ITALIC, 3),
        (Attrs::UNDERLINE, 4),
        (Attrs::REVERSE, 7),
    ] {
        if cell.attrs.contains(attr) {
            let _ = write!(out, ";{code}");
        }
    }
    write_color(out, cell.fg, depth, false);
    write_color(out, cell.bg, depth, true);
    out.push(b'm');
}

/// Turns a terminal event into an input event, or `None` for one the
/// scene has no use for (a key release, the terminal gaining focus).
///
/// Control-C is delivered as [`InputEvent::Close`]: in raw mode the
/// terminal no longer turns it into an interrupt, and a player must
/// always be able to leave.
pub fn translate(event: Event) -> Option<InputEvent> {
    let mods = |modifiers: KeyModifiers| {
        let mut mods = Mods::empty();
        mods.set(Mods::SHIFT, modifiers.contains(KeyModifiers::SHIFT));
        mods.set(Mods::CTRL, modifiers.contains(KeyModifiers::CONTROL));
        mods.set(Mods::ALT, modifiers.contains(KeyModifiers::ALT));
        mods
    };
    match event {
        Event::Key(key) => {
            if key.kind == KeyEventKind::Release {
                return None;
            }
            let mut held = mods(key.modifiers);
            let code = match key.code {
                event::KeyCode::Char(ch) => KeyCode::Char(ch),
                event::KeyCode::Enter => KeyCode::Enter,
                event::KeyCode::Esc => KeyCode::Escape,
                event::KeyCode::Backspace => KeyCode::Backspace,
                event::KeyCode::Tab => KeyCode::Tab,
                event::KeyCode::BackTab => {
                    held |= Mods::SHIFT;
                    KeyCode::Tab
                }
                event::KeyCode::Up => KeyCode::Up,
                event::KeyCode::Down => KeyCode::Down,
                event::KeyCode::Left => KeyCode::Left,
                event::KeyCode::Right => KeyCode::Right,
                event::KeyCode::Home => KeyCode::Home,
                event::KeyCode::End => KeyCode::End,
                event::KeyCode::PageUp => KeyCode::PageUp,
                event::KeyCode::PageDown => KeyCode::PageDown,
                event::KeyCode::Insert => KeyCode::Insert,
                event::KeyCode::Delete => KeyCode::Delete,
                event::KeyCode::F(n) if (1..=12).contains(&n) => KeyCode::F(n),
                _ => return None,
            };
            let chord = KeyChord::new(code, held);
            if chord == KeyChord::new(KeyCode::Char('c'), Mods::CTRL) {
                return Some(InputEvent::Close);
            }
            Some(InputEvent::Key(chord))
        }
        Event::Paste(text) => Some(InputEvent::Text(text)),
        Event::Resize(cols, rows) => Some(InputEvent::Resize { cols, rows }),
        Event::Mouse(mouse) => {
            let button = |button: event::MouseButton| match button {
                event::MouseButton::Left => MouseButton::Left,
                event::MouseButton::Right => MouseButton::Right,
                event::MouseButton::Middle => MouseButton::Middle,
            };
            let kind = match mouse.kind {
                MouseEventKind::Down(b) => MouseKind::Down(button(b)),
                MouseEventKind::Up(b) => MouseKind::Up(button(b)),
                MouseEventKind::Moved | MouseEventKind::Drag(_) => MouseKind::Move,
                MouseEventKind::ScrollUp => MouseKind::Wheel(-1),
                MouseEventKind::ScrollDown => MouseKind::Wheel(1),
                MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => return None,
            };
            Some(InputEvent::Mouse(MouseEvent {
                kind,
                col: mouse.column,
                row: mouse.row,
                mods: mods(mouse.modifiers),
            }))
        }
        Event::FocusGained | Event::FocusLost => None,
    }
}

/// Why the terminal frontend could not start.
#[derive(Debug)]
#[non_exhaustive]
pub enum TermError {
    /// Standard input or output is not a terminal: the program was
    /// started with its input or output redirected.
    NotATerminal,
    /// The terminal refused something.
    Io(io::Error),
}

impl core::fmt::Display for TermError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TermError::NotATerminal => {
                f.write_str("standard input and output must both be a terminal")
            }
            TermError::Io(error) => write!(f, "the terminal could not be set up: {error}"),
        }
    }
}

impl std::error::Error for TermError {}

impl From<io::Error> for TermError {
    fn from(error: io::Error) -> TermError {
        TermError::Io(error)
    }
}

/// Gives the terminal back: leaves raw mode and the alternate screen,
/// shows the cursor and stops reporting the mouse and pastes. Safe to
/// call more than once, and when the terminal was never taken.
pub fn restore_terminal() {
    let _ = terminal::disable_raw_mode();
    let mut out = io::stdout();
    if !out.is_terminal() {
        // Nothing was taken, and escape sequences would only end up in a
        // file or a pipe.
        return;
    }
    let _ = execute!(
        out,
        event::DisableBracketedPaste,
        event::DisableMouseCapture,
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = out.flush();
}

/// Makes a panic anywhere restore the terminal before its message is
/// printed. Installed once, however many frontends are started.
fn restore_on_panic() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

/// The terminal as a [`Frontend`]. It owns the terminal from
/// [`TermFrontend::start`] until it is dropped.
#[derive(Debug)]
pub struct TermFrontend {
    renderer: Renderer,
    mouse: bool,
    size: (u16, u16),
    /// Set when reading input fails, so the failure is reported once.
    broken: bool,
    last_error: Option<io::Error>,
}

impl TermFrontend {
    /// Takes over the terminal: raw mode, the alternate screen, a hidden
    /// cursor, and mouse reports if `mouse` is true. Fails, leaving the
    /// terminal as it was, if input or output is not a terminal.
    pub fn start(mouse: bool) -> Result<TermFrontend, TermError> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(TermError::NotATerminal);
        }
        let depth = color_depth(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        );
        restore_on_panic();
        let set_up = || -> io::Result<(u16, u16)> {
            terminal::enable_raw_mode()?;
            let mut out = io::stdout();
            execute!(
                out,
                terminal::EnterAlternateScreen,
                cursor::Hide,
                event::EnableBracketedPaste
            )?;
            if mouse {
                execute!(out, event::EnableMouseCapture)?;
            }
            terminal::size()
        };
        match set_up() {
            Ok(size) => Ok(TermFrontend {
                renderer: Renderer::new(depth),
                mouse,
                size,
                broken: false,
                last_error: None,
            }),
            Err(error) => {
                restore_terminal();
                Err(TermError::Io(error))
            }
        }
    }

    /// The last error the terminal gave while showing a frame or reading
    /// input, if there was one since this was last asked.
    pub fn take_error(&mut self) -> Option<io::Error> {
        self.last_error.take()
    }
}

impl Drop for TermFrontend {
    fn drop(&mut self) {
        restore_terminal();
    }
}

impl Frontend for TermFrontend {
    fn caps(&self) -> Caps {
        Caps {
            subcell: false,
            zoom: false,
            alpha: false,
            mouse: self.mouse,
            color: self.renderer.depth(),
            cols: self.size.0,
            rows: self.size.1,
        }
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        if self.broken {
            return events;
        }
        loop {
            match event::poll(Duration::ZERO) {
                Ok(false) => break,
                Ok(true) => match event::read() {
                    Ok(read) => {
                        if let Event::Resize(cols, rows) = read {
                            self.size = (cols, rows);
                        }
                        events.extend(translate(read));
                    }
                    Err(error) => {
                        self.fail(error, &mut events);
                        break;
                    }
                },
                Err(error) => {
                    self.fail(error, &mut events);
                    break;
                }
            }
        }
        events
    }

    fn present(&mut self, frame: &Frame) {
        let mut out = io::stdout().lock();
        if let Err(error) = self.renderer.render(frame, &mut out) {
            self.last_error = Some(error);
        }
    }
}

impl TermFrontend {
    /// Input can no longer be read: say so once, as a request to close.
    fn fail(&mut self, error: io::Error, events: &mut Vec<InputEvent>) {
        self.broken = true;
        self.last_error = Some(error);
        events.push(InputEvent::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_scene::{Canvas, CellStyle, Rgb, Sprite};
    use crossterm::event::{KeyEvent, KeyEventState};

    fn rendered(renderer: &mut Renderer, frame: &Frame) -> String {
        let mut out = Vec::new();
        renderer.render(frame, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn style(fg: Color, bg: Color, attrs: Attrs) -> CellStyle {
        CellStyle { fg, bg, attrs }
    }

    #[test]
    fn the_first_frame_is_written_in_full() {
        let mut frame = Frame::new(3, 2);
        Canvas::new(&mut frame).print(0, 0, "ab", CellStyle::default());
        let mut renderer = Renderer::new(ColorDepth::Ansi16);
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[0m\x1b[2J\x1b[1;1H\x1b[0;37;40mab \x1b[2;1H   \x1b[0m"
        );
    }

    #[test]
    fn only_what_changed_is_written() {
        let mut frame = Frame::new(6, 3);
        let mut renderer = Renderer::new(ColorDepth::Ansi16);
        rendered(&mut renderer, &frame);
        assert_eq!(
            rendered(&mut renderer, &frame),
            "",
            "nothing changed, nothing written"
        );

        let red = style(
            Color::ansi(Ansi16::BrightRed),
            Color::ansi(Ansi16::Black),
            Attrs::BOLD,
        );
        let mut canvas = Canvas::new(&mut frame);
        canvas.print(1, 1, "hi", red);
        canvas.put(4, 1, '!', CellStyle::default());
        canvas.put(5, 2, '│', CellStyle::default());
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[2;2H\x1b[0;1;91;40mhi\x1b[2;5H\x1b[0;37;40m!\x1b[3;6H│\x1b[0m"
        );

        Canvas::new(&mut frame).put(2, 1, 'o', red);
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[2;3H\x1b[0;1;91;40mo\x1b[0m"
        );

        renderer.invalidate();
        assert!(rendered(&mut renderer, &frame).starts_with("\x1b[0m\x1b[2J"));
    }

    #[test]
    fn a_change_of_size_redraws_everything() {
        let mut renderer = Renderer::new(ColorDepth::Ansi16);
        rendered(&mut renderer, &Frame::new(2, 1));
        let text = rendered(&mut renderer, &Frame::new(3, 1));
        assert_eq!(text, "\x1b[0m\x1b[2J\x1b[1;1H\x1b[0;37;40m   \x1b[0m");
    }

    #[test]
    fn colours_map_to_what_the_terminal_has() {
        let crimson = Color::new(Rgb::new(0xa0, 0x18, 0x28), Ansi16::Magenta);
        let ink = Color::new(Rgb::new(0x10, 0x10, 0x10), Ansi16::Black);
        let mut frame = Frame::new(1, 1);
        Canvas::new(&mut frame).put(
            0,
            0,
            'x',
            style(crimson, ink, Attrs::UNDERLINE | Attrs::REVERSE),
        );
        let cell = |depth: ColorDepth| {
            let text = rendered(&mut Renderer::new(depth), &frame);
            text.strip_prefix("\x1b[0m\x1b[2J\x1b[1;1H")
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            cell(ColorDepth::TrueColor),
            "\x1b[0;4;7;38;2;160;24;40;48;2;16;16;16mx\x1b[0m"
        );
        assert_eq!(
            cell(ColorDepth::Ansi256),
            "\x1b[0;4;7;38;5;124;48;5;233mx\x1b[0m"
        );
        assert_eq!(
            cell(ColorDepth::Ansi16),
            "\x1b[0;4;7;35;40mx\x1b[0m",
            "with sixteen colours the definition's fallback is used, not the nearest colour"
        );
        assert_eq!(ansi16_code(Ansi16::BrightWhite, false), 97);
        assert_eq!(ansi16_code(Ansi16::BrightBlack, true), 100);
        assert_eq!(ansi16_code(Ansi16::White, true), 47);
    }

    #[test]
    fn sprites_snap_to_cells_and_effects_are_ignored() {
        let mut frame = Frame::new(4, 1);
        let mut renderer = Renderer::new(ColorDepth::Ansi16);
        rendered(&mut renderer, &frame);
        let green = Color::ansi(Ansi16::Green);
        frame.sprites.push(Sprite {
            scale: 3.0,
            alpha: 0.2,
            ..Sprite::new('*', green, 1.6, 0.2)
        });
        frame.fx.shake = 1.0;
        frame.fx.zoom = 2.0;
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[1;3H\x1b[0;32;40m*\x1b[0m"
        );
        assert_eq!(frame.sprites.len(), 1, "the frame given is not changed");

        frame.sprites[0].x = 2.7;
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[1;3H\x1b[0;37;40m \x1b[0;32;40m*\x1b[0m",
            "the sprite moved a cell: the old cell is cleared and the new one drawn"
        );
    }

    #[test]
    fn the_last_column_never_relies_on_where_the_cursor_ends_up() {
        let mut frame = Frame::new(2, 2);
        let mut renderer = Renderer::new(ColorDepth::Ansi16);
        rendered(&mut renderer, &frame);
        let mut canvas = Canvas::new(&mut frame);
        canvas.put(1, 0, 'a', CellStyle::default());
        canvas.put(0, 1, 'b', CellStyle::default());
        assert_eq!(
            rendered(&mut renderer, &frame),
            "\x1b[1;2H\x1b[0;37;40ma\x1b[2;1Hb\x1b[0m"
        );
    }

    #[test]
    fn colour_depth_comes_from_the_environment() {
        assert_eq!(
            color_depth(Some("truecolor"), Some("xterm")),
            ColorDepth::TrueColor
        );
        assert_eq!(color_depth(Some("24bit"), None), ColorDepth::TrueColor);
        assert_eq!(
            color_depth(None, Some("xterm-256color")),
            ColorDepth::Ansi256
        );
        assert_eq!(
            color_depth(Some("yes"), Some("screen-256color")),
            ColorDepth::Ansi256
        );
        assert_eq!(color_depth(None, Some("vt100")), ColorDepth::Ansi16);
        assert_eq!(color_depth(None, None), ColorDepth::Ansi16);
    }

    fn key(code: event::KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn keys_arrive_as_input_events() {
        let press = |code, modifiers| translate(key(code, modifiers, KeyEventKind::Press));
        let chord = |name: &str| Some(InputEvent::key(name).unwrap());
        assert_eq!(
            press(event::KeyCode::Char('h'), KeyModifiers::NONE),
            chord("h")
        );
        assert_eq!(
            press(event::KeyCode::Char('A'), KeyModifiers::SHIFT),
            chord("shift+a")
        );
        assert_eq!(
            press(event::KeyCode::Char('A'), KeyModifiers::NONE),
            chord("shift+a")
        );
        assert_eq!(
            press(event::KeyCode::Char('?'), KeyModifiers::SHIFT),
            chord("?")
        );
        assert_eq!(
            press(event::KeyCode::Char('s'), KeyModifiers::CONTROL),
            chord("ctrl+s")
        );
        assert_eq!(
            press(event::KeyCode::Up, KeyModifiers::ALT),
            chord("alt+up")
        );
        assert_eq!(
            press(event::KeyCode::Esc, KeyModifiers::NONE),
            chord("escape")
        );
        assert_eq!(
            press(event::KeyCode::Enter, KeyModifiers::NONE),
            chord("enter")
        );
        assert_eq!(
            press(event::KeyCode::BackTab, KeyModifiers::NONE),
            chord("shift+tab")
        );
        assert_eq!(
            press(event::KeyCode::BackTab, KeyModifiers::SHIFT),
            chord("shift+tab")
        );
        assert_eq!(
            press(event::KeyCode::PageDown, KeyModifiers::NONE),
            chord("page_down")
        );
        assert_eq!(press(event::KeyCode::F(5), KeyModifiers::NONE), chord("f5"));
        assert_eq!(press(event::KeyCode::F(20), KeyModifiers::NONE), None);
        assert_eq!(press(event::KeyCode::CapsLock, KeyModifiers::NONE), None);
        assert_eq!(
            press(event::KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(InputEvent::Close),
            "control-c always leaves"
        );
        assert_eq!(
            translate(key(
                event::KeyCode::Char('h'),
                KeyModifiers::NONE,
                KeyEventKind::Repeat
            )),
            chord("h")
        );
        assert_eq!(
            translate(key(
                event::KeyCode::Char('h'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            None
        );
    }

    #[test]
    fn resize_paste_and_mouse_arrive_as_input_events() {
        assert_eq!(
            translate(Event::Resize(100, 30)),
            Some(InputEvent::Resize {
                cols: 100,
                rows: 30
            })
        );
        assert_eq!(
            translate(Event::Paste("hello".into())),
            Some(InputEvent::Text("hello".into()))
        );
        assert_eq!(translate(Event::FocusGained), None);
        let mouse = |kind| {
            translate(Event::Mouse(event::MouseEvent {
                kind,
                column: 7,
                row: 3,
                modifiers: KeyModifiers::SHIFT,
            }))
        };
        let expect = |kind| {
            Some(InputEvent::Mouse(MouseEvent {
                kind,
                col: 7,
                row: 3,
                mods: Mods::SHIFT,
            }))
        };
        assert_eq!(
            mouse(MouseEventKind::Down(event::MouseButton::Left)),
            expect(MouseKind::Down(MouseButton::Left))
        );
        assert_eq!(
            mouse(MouseEventKind::Up(event::MouseButton::Right)),
            expect(MouseKind::Up(MouseButton::Right))
        );
        assert_eq!(mouse(MouseEventKind::Moved), expect(MouseKind::Move));
        assert_eq!(
            mouse(MouseEventKind::Drag(event::MouseButton::Left)),
            expect(MouseKind::Move)
        );
        assert_eq!(
            mouse(MouseEventKind::ScrollUp),
            expect(MouseKind::Wheel(-1))
        );
        assert_eq!(
            mouse(MouseEventKind::ScrollDown),
            expect(MouseKind::Wheel(1))
        );
        assert_eq!(mouse(MouseEventKind::ScrollLeft), None);
    }

    #[test]
    fn without_a_terminal_the_frontend_does_not_start() {
        // Under the test harness standard input is not a terminal, or
        // output is captured. Where both happen to be terminals this
        // test has nothing to check.
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            return;
        }
        let error = TermFrontend::start(false).unwrap_err();
        assert!(matches!(error, TermError::NotATerminal));
        assert_eq!(
            error.to_string(),
            "standard input and output must both be a terminal"
        );
    }
}
