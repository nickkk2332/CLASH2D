//! [`Shows`]: the `show` and `anim` definitions made ready for a theme.
//!
//! A `show` names colours and an animation; a theme says what the
//! colours are. [`UiDefs::shows`](crate::UiDefs::shows) resolves both
//! once, when the scene is given its definitions or its theme changes,
//! so that playing an event looks nothing up by name.

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

use super::def::{AnimDef, ShowDef, ShowId};
use super::log::Importance;
use crate::color::{Color, ColorRef};
use crate::glyph::{GlyphPart, Paint};
use crate::theme::Theme;

/// How long a show's own effects last unless it says: the glide of a
/// sprite it moves, its shake and its flash. In seconds.
pub const EFFECT_TIME_DEFAULT: f64 = 0.15;

/// One keyframe of an animation, with every part resolved: what the
/// frames before it gave and what it changes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AnimFrame {
    /// When the frame begins, as a share of the animation's time.
    pub at: f32,
    /// What is drawn from then on.
    pub part: GlyphPart,
}

/// An animation ready to play: an `anim` definition with its colours
/// resolved by a theme.
#[derive(Clone, PartialEq, Debug)]
pub struct Anim {
    /// How long it lasts, in seconds at the usual pace.
    pub time: f64,
    /// The keyframes, in order.
    pub frames: Vec<AnimFrame>,
    /// Whether the glyph travels from one place to another.
    pub travel: bool,
}

/// Why an animation cannot be used with a theme.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AnimError {
    /// A frame names a colour the theme does not know.
    #[error("unknown colour `{name}` in `frames[{frame}].fg`")]
    UnknownColor {
        /// The frame.
        frame: usize,
        /// The colour's name.
        name: String,
    },
}

impl AnimError {
    /// Where in the definition the mistake is.
    pub fn path(&self) -> String {
        match self {
            AnimError::UnknownColor { frame, .. } => format!("frames[{frame}].fg"),
        }
    }
}

/// The colour a definition names, as a theme knows it. `None` for a
/// name the theme does not know and for what only a glyph can name.
pub(crate) fn named_color(color: &ColorRef, theme: &Theme) -> Option<Color> {
    match color {
        ColorRef::Hex(rgb) => Some(Color::from_rgb(*rgb)),
        ColorRef::Named(name) => theme.color(name),
        ColorRef::Material | ColorRef::Own | ColorRef::Dress => None,
    }
}

impl Anim {
    /// Resolves an animation's colours through a theme. A colour the
    /// theme does not know is reported and left out, so that what can
    /// be drawn still is.
    pub fn compile(def: &AnimDef, theme: &Theme) -> (Anim, Vec<AnimError>) {
        let mut errors = Vec::new();
        let mut frames = Vec::with_capacity(def.frames.len());
        let mut so_far = GlyphPart::default();
        for (index, frame) in def.frames.iter().enumerate() {
            let fg = match &frame.fg {
                None => None,
                Some(color) => match named_color(color, theme) {
                    Some(color) => Some(Paint::Fixed(color)),
                    None => {
                        if let ColorRef::Named(name) = color {
                            errors.push(AnimError::UnknownColor {
                                frame: index,
                                name: name.clone(),
                            });
                        }
                        None
                    }
                },
            };
            let part = GlyphPart {
                ch: frame.ch,
                fg,
                bg: None,
                bold: frame.bold,
            };
            so_far = part.over(so_far);
            frames.push(AnimFrame {
                at: frame.at as f32,
                part: so_far,
            });
        }
        let anim = Anim {
            time: def.time.dur().0 as f64 / MICROS_PER_SECOND,
            frames,
            travel: def.travel,
        };
        (anim, errors)
    }

    /// What is drawn when a share of the animation's time has passed:
    /// the last frame that has begun.
    pub fn part_at(&self, share: f32) -> GlyphPart {
        self.frames
            .iter()
            .rev()
            .find(|frame| frame.at <= share)
            .or(self.frames.first())
            .map(|frame| frame.part)
            .unwrap_or_default()
    }
}

/// How many microseconds, the unit of a duration in data once read, a
/// second has.
pub(crate) const MICROS_PER_SECOND: f64 = 1_000_000.0;

/// A `show` definition ready to play under a theme.
#[derive(Clone, Debug)]
pub struct Show {
    /// The definition.
    pub id: ShowId,
    /// Its qualified id.
    pub name: String,
    /// Its animation, if it names one that could be made.
    pub anim: Option<Arc<Anim>>,
    /// The colour it flashes over the frame.
    pub flash: Option<Color>,
    /// How long its own effects last, in seconds.
    pub time: f64,
    /// How long it holds the queue, in seconds.
    pub min_time: f64,
    /// How much it matters.
    pub importance: Importance,
}

impl Show {
    /// A show with its times and importance from its definition, and
    /// no animation or flash yet.
    pub(crate) fn of(id: ShowId, name: &str, def: &ShowDef) -> Show {
        let seconds = |time: Option<clash_data::Time>| {
            time.map(|time| (time.dur().0.max(0)) as f64 / MICROS_PER_SECOND)
        };
        Show {
            id,
            name: name.to_owned(),
            anim: None,
            flash: None,
            time: seconds(def.time).unwrap_or(EFFECT_TIME_DEFAULT),
            min_time: seconds(def.min_time).unwrap_or(0.0),
            importance: def.importance,
        }
    }
}

/// Every `show` that can be played, by the name of its event, in order
/// of qualified id.
#[derive(Clone, Debug, Default)]
pub struct Shows {
    by_event: BTreeMap<String, Vec<Show>>,
}

impl Shows {
    /// No shows at all: every event is applied silently.
    pub fn new() -> Shows {
        Shows::default()
    }

    /// Adds a show for an event.
    pub(crate) fn add(&mut self, event: &str, show: Show) {
        self.by_event
            .entry(event.to_owned())
            .or_default()
            .push(show);
    }

    /// The shows about an event, in order of qualified id.
    pub fn for_event(&self, event: &str) -> &[Show] {
        self.by_event.get(event).map_or(&[], Vec::as_slice)
    }

    /// The names of the events some show is about, in order.
    pub fn events(&self) -> impl Iterator<Item = &str> {
        self.by_event.keys().map(String::as_str)
    }

    /// How many shows there are.
    pub fn len(&self) -> usize {
        self.by_event.values().map(Vec::len).sum()
    }

    /// Whether there is no show.
    pub fn is_empty(&self) -> bool {
        self.by_event.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Ansi16;

    #[test]
    fn frames_keep_what_earlier_frames_gave() {
        let def: AnimDef = serde_json::from_str(
            r#"{ "time": 300000, "frames": [ { "ch": "*", "fg": "red", "bold": true },
                 { "at": 0.5, "ch": "%" }, { "at": 0.75, "fg": "octarine" } ] }"#,
        )
        .unwrap();
        let (anim, errors) = Anim::compile(&def, &Theme::default());
        assert_eq!(
            errors,
            [AnimError::UnknownColor {
                frame: 2,
                name: "octarine".to_owned()
            }]
        );
        assert_eq!(errors[0].path(), "frames[2].fg");
        assert!((anim.time - 0.3).abs() < 1e-9);
        let red = Some(Paint::Fixed(Color::ansi(Ansi16::Red)));
        assert_eq!(anim.part_at(0.0).ch, Some('*'));
        assert_eq!(anim.part_at(0.49).fg, red);
        let late = anim.part_at(0.6);
        assert_eq!((late.ch, late.fg, late.bold), (Some('%'), red, Some(true)));
        // The unknown colour is left out and the frame keeps the red.
        assert_eq!(anim.part_at(1.0).fg, red);
    }
}
