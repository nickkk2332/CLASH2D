//! What the window delivers turned into the scene's input (12 §9.1):
//! [`InputMapper`].
//!
//! Keys get the names the terminal gives them, with the same modifiers:
//! a character key is its character, a capital is `shift+` its letter,
//! and shift is not named with any other character; the named keys are
//! those of 12 §9.1. A key held down repeats as presses; a release is
//! nothing. Unlike the terminal, Control-C is an ordinary key: a window
//! is left by closing it, which arrives as [`InputEvent::Close`].
//!
//! The mouse is reported with the cell under the pointer, worked out
//! from the [`Layout`] of the last picture shown and its zoom. A move
//! is reported only when the pointer reaches another cell, and nothing
//! is reported while it is off the grid. The wheel is counted in
//! notches, negative toward the top; a touchpad's smooth scrolling is
//! added up until it makes a notch, a cell's height of movement each.
//!
//! The mapping is kept apart from the window so that it is tested
//! without one.

use clash_scene::{InputEvent, KeyChord, KeyCode, Mods, MouseButton, MouseEvent, MouseKind};
use winit::event::{ElementState, MouseScrollDelta};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::draw::Layout;

/// The scene's key for a key the window reports, without modifiers, or
/// `None` for a key the scene has no name for.
pub fn key_code(key: &Key) -> Option<KeyCode> {
    Some(match key {
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if !ch.is_control() => KeyCode::Char(ch),
                _ => return None,
            }
        }
        Key::Named(named) => match named {
            NamedKey::Space => KeyCode::Char(' '),
            NamedKey::Enter => KeyCode::Enter,
            NamedKey::Escape => KeyCode::Escape,
            NamedKey::Backspace => KeyCode::Backspace,
            NamedKey::Tab => KeyCode::Tab,
            NamedKey::ArrowUp => KeyCode::Up,
            NamedKey::ArrowDown => KeyCode::Down,
            NamedKey::ArrowLeft => KeyCode::Left,
            NamedKey::ArrowRight => KeyCode::Right,
            NamedKey::Home => KeyCode::Home,
            NamedKey::End => KeyCode::End,
            NamedKey::PageUp => KeyCode::PageUp,
            NamedKey::PageDown => KeyCode::PageDown,
            NamedKey::Insert => KeyCode::Insert,
            NamedKey::Delete => KeyCode::Delete,
            NamedKey::F1 => KeyCode::F(1),
            NamedKey::F2 => KeyCode::F(2),
            NamedKey::F3 => KeyCode::F(3),
            NamedKey::F4 => KeyCode::F(4),
            NamedKey::F5 => KeyCode::F(5),
            NamedKey::F6 => KeyCode::F(6),
            NamedKey::F7 => KeyCode::F(7),
            NamedKey::F8 => KeyCode::F(8),
            NamedKey::F9 => KeyCode::F(9),
            NamedKey::F10 => KeyCode::F(10),
            NamedKey::F11 => KeyCode::F(11),
            NamedKey::F12 => KeyCode::F(12),
            _ => return None,
        },
        _ => return None,
    })
}

/// The scene's modifiers for the window's.
pub fn mods(state: ModifiersState) -> Mods {
    let mut mods = Mods::empty();
    mods.set(Mods::SHIFT, state.shift_key());
    mods.set(Mods::CTRL, state.control_key());
    mods.set(Mods::ALT, state.alt_key());
    mods
}

/// The scene's mouse button for the window's; the side buttons and the
/// rest have none.
pub fn mouse_button(button: winit::event::MouseButton) -> Option<MouseButton> {
    match button {
        winit::event::MouseButton::Left => Some(MouseButton::Left),
        winit::event::MouseButton::Right => Some(MouseButton::Right),
        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Turns what the window reports into input events, keeping what it
/// must between them: the modifiers held, where the pointer is, and the
/// part of a notch the wheel has turned. See the [module](self)
/// documentation.
#[derive(Clone, Debug, Default)]
pub struct InputMapper {
    held: ModifiersState,
    /// The cell under the pointer, if it is over the grid.
    cell: Option<(u16, u16)>,
    /// The pointer's place in pixels, if it is in the window.
    pointer: Option<(f64, f64)>,
    /// Wheel movement not yet a whole notch, in notches.
    wheel: f64,
}

impl InputMapper {
    /// A mapper with nothing held and the pointer outside the window.
    pub fn new() -> InputMapper {
        InputMapper::default()
    }

    /// The modifier keys held changed.
    pub fn modifiers(&mut self, held: ModifiersState) {
        self.held = held;
    }

    /// The window gained or lost the keyboard. Losing it forgets the
    /// modifiers, whose release the window will not hear of.
    pub fn focus(&mut self, focused: bool) {
        if !focused {
            self.held = ModifiersState::empty();
        }
    }

    /// A key went down or came up. A press is a key event; a key
    /// written as more than one character, as some layouts and input
    /// methods give, arrives as text instead.
    pub fn key(&self, key: &Key, state: ElementState) -> Option<InputEvent> {
        if state != ElementState::Pressed {
            return None;
        }
        if let Some(code) = key_code(key) {
            return Some(InputEvent::Key(KeyChord::new(code, mods(self.held))));
        }
        match key {
            Key::Character(text) if text.chars().count() > 1 => {
                Some(InputEvent::Text(text.to_string()))
            }
            _ => None,
        }
    }

    /// Text an input method composed.
    pub fn text(&self, text: &str) -> Option<InputEvent> {
        (!text.is_empty()).then(|| InputEvent::Text(text.to_owned()))
    }

    /// The pointer moved to a point of the window, in pixels from its
    /// top left, over a picture laid out as `layout` and drawn at
    /// `zoom`. Reported when it reaches another cell.
    pub fn moved(&mut self, x: f64, y: f64, layout: &Layout, zoom: f32) -> Option<InputEvent> {
        self.pointer = Some((x, y));
        let cell = layout.cell_under(x, y, zoom);
        let changed = cell.is_some() && cell != self.cell;
        self.cell = cell;
        changed.then(|| self.mouse(MouseKind::Move))?
    }

    /// The picture's layout or zoom changed under a pointer that has
    /// not moved: what is under it now, if that is another cell.
    pub fn relaid(&mut self, layout: &Layout, zoom: f32) -> Option<InputEvent> {
        let (x, y) = self.pointer?;
        self.moved(x, y, layout, zoom)
    }

    /// The pointer left the window.
    pub fn left(&mut self) {
        self.pointer = None;
        self.cell = None;
    }

    /// A mouse button went down or came up, over the cell under the
    /// pointer.
    pub fn button(
        &mut self,
        state: ElementState,
        button: winit::event::MouseButton,
    ) -> Option<InputEvent> {
        let button = mouse_button(button)?;
        self.mouse(match state {
            ElementState::Pressed => MouseKind::Down(button),
            ElementState::Released => MouseKind::Up(button),
        })
    }

    /// The wheel turned. `cell_height` is how many pixels of smooth
    /// scrolling make a notch.
    pub fn wheel(&mut self, delta: MouseScrollDelta, cell_height: f64) -> Option<InputEvent> {
        // The window counts toward the top as positive.
        self.wheel -= match delta {
            MouseScrollDelta::LineDelta(_, lines) => f64::from(lines),
            MouseScrollDelta::PixelDelta(moved) => moved.y / cell_height.max(1.0),
        };
        if !self.wheel.is_finite() {
            self.wheel = 0.0;
        }
        let notches = self.wheel.trunc();
        if notches == 0.0 {
            return None;
        }
        self.wheel -= notches;
        let notches = notches.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
        self.mouse(MouseKind::Wheel(notches))
    }

    /// A mouse event over the cell under the pointer, if there is one.
    fn mouse(&self, kind: MouseKind) -> Option<InputEvent> {
        let (col, row) = self.cell?;
        Some(InputEvent::Mouse(MouseEvent {
            kind,
            col,
            row,
            mods: mods(self.held),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::GlyphAtlas;
    use crate::atlas::tests::font_bytes;
    use winit::dpi::PhysicalPosition;
    use winit::keyboard::SmolStr;

    fn character(text: &str) -> Key {
        Key::Character(SmolStr::new(text))
    }

    fn press(mapper: &InputMapper, key: Key) -> Option<InputEvent> {
        mapper.key(&key, ElementState::Pressed)
    }

    fn chord(name: &str) -> Option<InputEvent> {
        Some(InputEvent::key(name).unwrap())
    }

    fn held(state: ModifiersState) -> InputMapper {
        let mut mapper = InputMapper::new();
        mapper.modifiers(state);
        mapper
    }

    #[test]
    fn keys_get_the_names_the_terminal_gives_them() {
        let plain = InputMapper::new();
        let shift = held(ModifiersState::SHIFT);
        let ctrl = held(ModifiersState::CONTROL);
        let alt = held(ModifiersState::ALT);
        assert_eq!(press(&plain, character("h")), chord("h"));
        assert_eq!(press(&shift, character("A")), chord("shift+a"));
        // Caps lock: a capital with no shift held is still `shift+a`.
        assert_eq!(press(&plain, character("A")), chord("shift+a"));
        assert_eq!(press(&shift, character("?")), chord("?"));
        assert_eq!(press(&shift, character("!")), chord("!"));
        assert_eq!(press(&ctrl, character("s")), chord("ctrl+s"));
        assert_eq!(press(&alt, Key::Named(NamedKey::ArrowUp)), chord("alt+up"));
        assert_eq!(press(&plain, Key::Named(NamedKey::Escape)), chord("escape"));
        assert_eq!(press(&plain, Key::Named(NamedKey::Enter)), chord("enter"));
        assert_eq!(press(&plain, Key::Named(NamedKey::Space)), chord("space"));
        assert_eq!(
            press(&shift, Key::Named(NamedKey::Space)),
            chord("shift+space")
        );
        assert_eq!(press(&shift, Key::Named(NamedKey::Tab)), chord("shift+tab"));
        assert_eq!(
            press(&plain, Key::Named(NamedKey::PageDown)),
            chord("page_down")
        );
        assert_eq!(press(&plain, Key::Named(NamedKey::F5)), chord("f5"));
        assert_eq!(press(&plain, Key::Named(NamedKey::F12)), chord("f12"));
        assert_eq!(press(&plain, Key::Named(NamedKey::F13)), None);
        assert_eq!(press(&plain, Key::Named(NamedKey::CapsLock)), None);
        assert_eq!(press(&plain, Key::Named(NamedKey::Shift)), None);
        assert_eq!(press(&plain, character("é")), chord("é"));
        let both = held(ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(
            press(&both, Key::Named(NamedKey::Delete)),
            chord("ctrl+shift+delete")
        );
        assert_eq!(
            press(&held(ModifiersState::SUPER), character("q")),
            chord("q")
        );
    }

    #[test]
    fn control_c_is_a_key_and_releases_are_nothing() {
        let ctrl = held(ModifiersState::CONTROL);
        assert_eq!(press(&ctrl, character("c")), chord("ctrl+c"));
        assert_eq!(ctrl.key(&character("c"), ElementState::Released), None);
    }

    #[test]
    fn text_is_what_did_not_arrive_as_one_key() {
        let mapper = InputMapper::new();
        assert_eq!(
            press(&mapper, character("ﬃx")),
            Some(InputEvent::Text("ﬃx".to_owned()))
        );
        assert_eq!(
            mapper.text("日本"),
            Some(InputEvent::Text("日本".to_owned()))
        );
        assert_eq!(mapper.text(""), None);
        assert_eq!(press(&mapper, character("\u{7}")), None);
    }

    #[test]
    fn losing_the_focus_forgets_the_modifiers() {
        let mut mapper = held(ModifiersState::CONTROL);
        mapper.focus(false);
        assert_eq!(press(&mapper, character("s")), chord("s"));
    }

    fn layout() -> Layout {
        let atlas = GlyphAtlas::from_bytes(&font_bytes(), 16).unwrap();
        // Ten by five cells of 8 by 16, at (2, 2) in an 84 by 84 window.
        Layout::new(10, 5, &atlas, 84, 84)
    }

    fn mouse(kind: MouseKind, col: u16, row: u16, mods: Mods) -> Option<InputEvent> {
        Some(InputEvent::Mouse(MouseEvent {
            kind,
            col,
            row,
            mods,
        }))
    }

    #[test]
    fn the_mouse_is_reported_with_the_cell_under_it() {
        let layout = layout();
        let mut mapper = InputMapper::new();
        let left = winit::event::MouseButton::Left;
        assert_eq!(
            mapper.button(ElementState::Pressed, left),
            None,
            "nowhere yet"
        );
        assert_eq!(mapper.moved(0.0, 0.0, &layout, 1.0), None, "off the grid");
        assert_eq!(
            mapper.moved(11.0, 20.5, &layout, 1.0),
            mouse(MouseKind::Move, 1, 1, Mods::empty())
        );
        assert_eq!(
            mapper.moved(16.0, 30.0, &layout, 1.0),
            None,
            "the same cell"
        );
        assert_eq!(
            mapper.moved(18.0, 30.0, &layout, 1.0),
            mouse(MouseKind::Move, 2, 1, Mods::empty())
        );
        mapper.modifiers(ModifiersState::SHIFT);
        assert_eq!(
            mapper.button(ElementState::Pressed, left),
            mouse(MouseKind::Down(MouseButton::Left), 2, 1, Mods::SHIFT)
        );
        assert_eq!(
            mapper.button(ElementState::Released, winit::event::MouseButton::Right),
            mouse(MouseKind::Up(MouseButton::Right), 2, 1, Mods::SHIFT)
        );
        assert_eq!(
            mapper.button(ElementState::Pressed, winit::event::MouseButton::Back),
            None
        );
        // Zoomed in twice about the centre (42, 42), the same point is
        // over another cell.
        assert_eq!(
            mapper.relaid(&layout, 2.0),
            mouse(MouseKind::Move, 3, 2, Mods::SHIFT)
        );
        mapper.left();
        assert_eq!(mapper.button(ElementState::Pressed, left), None);
    }

    #[test]
    fn the_wheel_turns_in_notches() {
        let layout = layout();
        let mut mapper = InputMapper::new();
        mapper.moved(40.0, 40.0, &layout, 1.0);
        let (col, row) = (4, 2);
        assert_eq!(
            mapper.wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 16.0),
            mouse(MouseKind::Wheel(-1), col, row, Mods::empty()),
            "toward the top"
        );
        assert_eq!(
            mapper.wheel(MouseScrollDelta::LineDelta(0.0, -3.0), 16.0),
            mouse(MouseKind::Wheel(3), col, row, Mods::empty())
        );
        let smooth = |y| MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y));
        assert_eq!(mapper.wheel(smooth(-10.0), 16.0), None);
        assert_eq!(
            mapper.wheel(smooth(-10.0), 16.0),
            mouse(MouseKind::Wheel(1), col, row, Mods::empty()),
            "twenty pixels down is a notch and a bit"
        );
        assert_eq!(mapper.wheel(smooth(-8.0), 16.0), None);
        assert_eq!(mapper.wheel(smooth(f64::NAN), 16.0), None);
    }
}
