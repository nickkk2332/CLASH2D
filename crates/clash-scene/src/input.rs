//! Input: what a frontend delivers ([`InputEvent`], 12 §9.1), how a key
//! is written in data ([`KeyChord`]), and the `keymap` definition that
//! names what a key means in a context (12 §9.2).
//!
//! # Key names in data
//!
//! A key is written as its character (`"h"`, `"?"`, `"3"`) or one of the
//! names `enter`, `escape`, `backspace`, `tab`, `space`, `up`, `down`,
//! `left`, `right`, `home`, `end`, `page_up`, `page_down`, `insert`,
//! `delete` and `f1` to `f12`, with any of `shift+`, `ctrl+` and `alt+` in
//! front: `"shift+a"`, `"ctrl+page_down"`.
//!
//! A letter is always written in lower case, and `shift+a` is the capital.
//! For a character that is not a letter the shift key is not named: `"?"`
//! is whatever key combination types a question mark.
//!
//! # Keys and text
//!
//! A frontend reports a key press as [`InputEvent::Key`], including keys
//! that type a character. It reports [`InputEvent::Text`] only for text
//! that did not arrive as key presses: a paste, or the result of an input
//! method. It never reports the same keystroke as both.

use core::fmt;
use std::collections::BTreeMap;

use bitflags::bitflags;
use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};
use thiserror::Error;

bitflags! {
    /// The modifier keys held with a key or a mouse button.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
    pub struct Mods: u8 {
        /// Shift.
        const SHIFT = 1;
        /// Control.
        const CTRL = 2;
        /// Alt (Option).
        const ALT = 4;
    }
}

/// Which key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum KeyCode {
    /// A key that types a character. Letters are lower case; the capital
    /// is the letter with [`Mods::SHIFT`].
    Char(char),
    /// Enter or Return.
    Enter,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// Tab. Shift-Tab is this with [`Mods::SHIFT`].
    Tab,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// A function key, 1 to 12.
    F(u8),
}

/// A key with its modifiers: one key press, and one key as data names it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct KeyChord {
    /// Which key.
    pub code: KeyCode,
    /// The modifiers held.
    pub mods: Mods,
}

/// Why a text does not name a key.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The text is not a key name.
    #[error(
        "`{0}` is not a key: write a character, or a name such as `enter`, `up` or `f1`, with `shift+`, `ctrl+` or `alt+` in front"
    )]
    Unknown(String),
    /// A modifier was named twice.
    #[error("`{0}` names the same modifier twice")]
    RepeatedModifier(String),
}

impl KeyChord {
    /// A key with modifiers, brought to the one form each key has: a
    /// letter is lower case with shift held for the capital, and shift is
    /// dropped from any other character, since the character already says
    /// it.
    pub fn new(code: KeyCode, mods: Mods) -> KeyChord {
        match code {
            KeyCode::Char(ch) if ch.is_alphabetic() => {
                let lower = ch.to_lowercase().next().unwrap_or(ch);
                let mods = if ch.is_uppercase() {
                    mods | Mods::SHIFT
                } else {
                    mods
                };
                KeyChord {
                    code: KeyCode::Char(lower),
                    mods,
                }
            }
            // Shift with the space bar types nothing different, so it is
            // kept and can be bound.
            KeyCode::Char(' ') => KeyChord { code, mods },
            KeyCode::Char(ch) => KeyChord {
                code: KeyCode::Char(ch),
                mods: mods - Mods::SHIFT,
            },
            code => KeyChord { code, mods },
        }
    }

    /// A key with no modifiers.
    pub fn plain(code: KeyCode) -> KeyChord {
        KeyChord::new(code, Mods::empty())
    }

    /// A character key as typed: `'A'` is `shift+a`.
    pub fn char(ch: char) -> KeyChord {
        KeyChord::new(KeyCode::Char(ch), Mods::empty())
    }

    /// The character this key press types, if it types one: a character
    /// key with neither control nor alt held.
    pub fn typed(self) -> Option<char> {
        match self.code {
            KeyCode::Char(ch) if !self.mods.intersects(Mods::CTRL | Mods::ALT) => {
                if self.mods.contains(Mods::SHIFT) {
                    Some(ch.to_uppercase().next().unwrap_or(ch))
                } else {
                    Some(ch)
                }
            }
            _ => None,
        }
    }

    /// Reads a key as data writes it. See the module documentation.
    pub fn parse(text: &str) -> Result<KeyChord, KeyError> {
        let unknown = || KeyError::Unknown(text.to_owned());
        let mut mods = Mods::empty();
        let mut rest = text;
        // A lone "+" is the plus key, and "ctrl++" is control with it.
        while let Some((word, tail)) = rest.split_once('+') {
            if tail.is_empty() {
                break;
            }
            let modifier = match word {
                "shift" => Mods::SHIFT,
                "ctrl" => Mods::CTRL,
                "alt" => Mods::ALT,
                _ => return Err(unknown()),
            };
            if mods.contains(modifier) {
                return Err(KeyError::RepeatedModifier(text.to_owned()));
            }
            mods |= modifier;
            rest = tail;
        }
        let code = match rest {
            "enter" => KeyCode::Enter,
            "escape" => KeyCode::Escape,
            "backspace" => KeyCode::Backspace,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "page_up" => KeyCode::PageUp,
            "page_down" => KeyCode::PageDown,
            "insert" => KeyCode::Insert,
            "delete" => KeyCode::Delete,
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) if !ch.is_control() && !ch.is_uppercase() && ch != ' ' => {
                        KeyCode::Char(ch)
                    }
                    _ => match other.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                        Some(n) if (1..=12).contains(&n) && !other.starts_with("f0") => {
                            KeyCode::F(n)
                        }
                        _ => return Err(unknown()),
                    },
                }
            }
        };
        if matches!(code, KeyCode::Char(ch) if !ch.is_alphabetic() && ch != ' ')
            && mods.contains(Mods::SHIFT)
        {
            // "shift+?" has no single meaning across keyboards.
            return Err(unknown());
        }
        Ok(KeyChord { code, mods })
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (Mods::CTRL, "ctrl+"),
            (Mods::ALT, "alt+"),
            (Mods::SHIFT, "shift+"),
        ] {
            if self.mods.contains(flag) {
                f.write_str(name)?;
            }
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Char(ch) => write!(f, "{ch}"),
            KeyCode::Enter => f.write_str("enter"),
            KeyCode::Escape => f.write_str("escape"),
            KeyCode::Backspace => f.write_str("backspace"),
            KeyCode::Tab => f.write_str("tab"),
            KeyCode::Up => f.write_str("up"),
            KeyCode::Down => f.write_str("down"),
            KeyCode::Left => f.write_str("left"),
            KeyCode::Right => f.write_str("right"),
            KeyCode::Home => f.write_str("home"),
            KeyCode::End => f.write_str("end"),
            KeyCode::PageUp => f.write_str("page_up"),
            KeyCode::PageDown => f.write_str("page_down"),
            KeyCode::Insert => f.write_str("insert"),
            KeyCode::Delete => f.write_str("delete"),
            KeyCode::F(n) => write!(f, "f{n}"),
        }
    }
}

impl KeyChord {
    /// The key as a person reads it (decision D-866): `Shift+J`, `Tab`,
    /// `F1`, `Esc`, `Page Up`, `Ctrl+D`, `Space`, `,`. A keymap writes the
    /// same key `shift+j`, `tab`, `f1`, `escape`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (flag, name) in [
            (Mods::CTRL, "Ctrl+"),
            (Mods::ALT, "Alt+"),
            (Mods::SHIFT, "Shift+"),
        ] {
            if self.mods.contains(flag) {
                out.push_str(name);
            }
        }
        let key = match self.code {
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char(ch) if self.mods.contains(Mods::SHIFT) => {
                ch.to_uppercase().collect::<String>()
            }
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::Enter => "Enter".to_owned(),
            KeyCode::Escape => "Esc".to_owned(),
            KeyCode::Backspace => "Backspace".to_owned(),
            KeyCode::Tab => "Tab".to_owned(),
            KeyCode::Up => "Up".to_owned(),
            KeyCode::Down => "Down".to_owned(),
            KeyCode::Left => "Left".to_owned(),
            KeyCode::Right => "Right".to_owned(),
            KeyCode::Home => "Home".to_owned(),
            KeyCode::End => "End".to_owned(),
            KeyCode::PageUp => "Page Up".to_owned(),
            KeyCode::PageDown => "Page Down".to_owned(),
            KeyCode::Insert => "Insert".to_owned(),
            KeyCode::Delete => "Delete".to_owned(),
            KeyCode::F(n) => format!("F{n}"),
        };
        out.push_str(&key);
        out
    }
}

impl<'de> Deserialize<'de> for KeyChord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<KeyChord, D::Error> {
        struct ChordVisitor;
        impl Visitor<'_> for ChordVisitor {
            type Value = KeyChord;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a key such as `h`, `shift+a` or `enter`")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<KeyChord, E> {
                KeyChord::parse(text).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(ChordVisitor)
    }
}

/// A mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton {
    /// The main button.
    Left,
    /// The second button.
    Right,
    /// The wheel pressed as a button.
    Middle,
}

/// What the mouse did.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseKind {
    /// The pointer moved.
    Move,
    /// A button went down.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// The wheel turned: negative toward the top of the content, positive
    /// toward its end, in notches.
    Wheel(i16),
}

/// A mouse event, with the cell under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MouseEvent {
    /// What happened.
    pub kind: MouseKind,
    /// The column of the cell under the pointer.
    pub col: u16,
    /// The row of the cell under the pointer.
    pub row: u16,
    /// The modifier keys held.
    pub mods: Mods,
}

/// Something a frontend delivers to the scene (12 §9.1).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum InputEvent {
    /// A key was pressed, or repeated while held.
    Key(KeyChord),
    /// Text arrived that was not key presses: a paste or an input method.
    Text(String),
    /// The mouse did something.
    Mouse(MouseEvent),
    /// The grid changed size.
    Resize {
        /// The new width in cells.
        cols: u16,
        /// The new height in cells.
        rows: u16,
    },
    /// The player asked to close the window.
    Close,
}

impl InputEvent {
    /// A key press, from the key as data writes it. For tests and
    /// scripted input.
    pub fn key(name: &str) -> Result<InputEvent, KeyError> {
        KeyChord::parse(name).map(InputEvent::Key)
    }
}

/// The context whose names the interface toolkit navigates by, unless a
/// screen names another.
pub const CONTEXT_MENU: &str = "menu";

/// A `keymap` definition (12 §9.2): for each context, what each key
/// means. The meanings are names; what a name does is for controls and
/// screens to say.
///
/// ```json
/// { "type": "keymap", "id": "default",
///   "map":  { "h": "west", "shift+a": "attack_precise" },
///   "menu": { "up": "prev", "down": "next", "enter": "accept", "escape": "back" } }
/// ```
///
/// A context is whatever a screen names as its own; a key a context
/// does not name is looked up in `menu`. What a name does is said by
/// the screen on top, by a `control` ([`crate::ControlDef`]) or by the
/// toolkit, in that order. [`Scene::rebind`](crate::Scene::rebind)
/// changes the keymap in force. The `skip` name comes with playback.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(transparent)]
pub struct KeymapDef {
    /// For each context, the name each key has in it.
    pub contexts: BTreeMap<String, BTreeMap<KeyChord, String>>,
}

impl KeymapDef {
    /// What a key means in a context, if the keymap says.
    pub fn name(&self, context: &str, key: KeyChord) -> Option<&str> {
        self.contexts.get(context)?.get(&key).map(String::as_str)
    }

    /// A keymap with one context holding the engine's fallback keys for
    /// the toolkit's navigation names (see [`crate::ui`]): arrows, Enter,
    /// Escape, Tab and the rest of a keyboard's navigation keys. It is
    /// what a profile with no keymap gets, so that its screens can still
    /// be used.
    pub fn fallback() -> KeymapDef {
        let pairs = [
            ("up", "prev"),
            ("down", "next"),
            ("left", "left"),
            ("right", "right"),
            ("enter", "accept"),
            ("escape", "back"),
            ("tab", "focus_next"),
            ("shift+tab", "focus_prev"),
            ("page_up", "page_prev"),
            ("page_down", "page_next"),
            ("home", "first"),
            ("end", "last"),
            ("space", "toggle"),
            ("backspace", "erase"),
            ("delete", "delete"),
        ];
        let menu = pairs
            .into_iter()
            .filter_map(|(key, name)| Some((KeyChord::parse(key).ok()?, name.to_owned())))
            .collect();
        KeymapDef {
            contexts: BTreeMap::from([(CONTEXT_MENU.to_owned(), menu)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_as_data_writes_them() {
        let key = |text: &str| KeyChord::parse(text).unwrap();
        assert_eq!(
            key("h"),
            KeyChord {
                code: KeyCode::Char('h'),
                mods: Mods::empty()
            }
        );
        assert_eq!(
            key("shift+a"),
            KeyChord {
                code: KeyCode::Char('a'),
                mods: Mods::SHIFT
            }
        );
        assert_eq!(key("ctrl+alt+page_down").mods, Mods::CTRL | Mods::ALT);
        assert_eq!(key("space").code, KeyCode::Char(' '));
        assert_eq!(key("+").code, KeyCode::Char('+'));
        assert_eq!(
            key("ctrl++"),
            KeyChord {
                code: KeyCode::Char('+'),
                mods: Mods::CTRL
            }
        );
        assert_eq!(key("f12").code, KeyCode::F(12));
        assert_eq!(key("f").code, KeyCode::Char('f'));
        assert_eq!(key("?").code, KeyCode::Char('?'));
        for bad in [
            "",
            "A",
            "f13",
            "f0",
            "f01",
            "return",
            "shift+",
            "hyper+a",
            "shift+shift+a",
            "shift+?",
            "ab",
            " ",
        ] {
            assert!(KeyChord::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn every_key_displays_as_it_parses() {
        for text in [
            "h",
            "shift+a",
            "ctrl+alt+shift+f5",
            "space",
            "enter",
            "page_up",
            "alt+?",
            "ctrl++",
            "é",
        ] {
            assert_eq!(KeyChord::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn a_frontend_key_is_brought_to_the_same_form_as_data() {
        assert_eq!(KeyChord::char('A'), KeyChord::parse("shift+a").unwrap());
        assert_eq!(
            KeyChord::new(KeyCode::Char('A'), Mods::SHIFT),
            KeyChord::parse("shift+a").unwrap()
        );
        assert_eq!(
            KeyChord::new(KeyCode::Char('?'), Mods::SHIFT),
            KeyChord::parse("?").unwrap()
        );
        assert_eq!(KeyChord::char('A').typed(), Some('A'));
        assert_eq!(KeyChord::char('x').typed(), Some('x'));
        assert_eq!(KeyChord::new(KeyCode::Char('x'), Mods::CTRL).typed(), None);
        assert_eq!(KeyChord::plain(KeyCode::Enter).typed(), None);
    }

    #[test]
    fn a_keymap_names_keys_by_context() {
        let keymap: KeymapDef = serde_json::from_str(
            r#"{ "map":  { "h": "west", "a": "attack", "shift+a": "attack_precise" },
                 "menu": { "up": "prev", "down": "next", "enter": "accept", "escape": "back" } }"#,
        )
        .unwrap();
        assert_eq!(
            keymap.name("map", KeyChord::char('A')),
            Some("attack_precise")
        );
        assert_eq!(keymap.name("map", KeyChord::char('a')), Some("attack"));
        assert_eq!(
            keymap.name("menu", KeyChord::plain(KeyCode::Escape)),
            Some("back")
        );
        assert_eq!(keymap.name("menu", KeyChord::char('h')), None);
        assert_eq!(keymap.name("nowhere", KeyChord::char('h')), None);

        let err =
            serde_json::from_str::<KeymapDef>(r#"{ "menu": { "retrun": "accept" } }"#).unwrap_err();
        assert!(err.to_string().contains("`retrun` is not a key"), "{err}");
    }

    #[test]
    fn the_fallback_keymap_covers_navigation() {
        let keymap = KeymapDef::fallback();
        assert_eq!(keymap.contexts[CONTEXT_MENU].len(), 15);
        assert_eq!(
            keymap.name(CONTEXT_MENU, KeyChord::parse("shift+tab").unwrap()),
            Some("focus_prev")
        );
    }
}
