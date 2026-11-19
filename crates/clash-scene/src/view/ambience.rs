//! Ambience: how a condition looks (12 §6.4).
//!
//! An [`AmbienceDef`] is what a mod writes: a tint, a dimming and
//! particles. It is named by a weather state, a medium, a sky or an area
//! kind, and the scene draws the ambience of where the viewed character
//! is. [`Ambience::compile`] resolves its colour through a theme.
//!
//! Ambience is presentation only: nothing here is read by the
//! simulation.
//!
//! # What is drawn
//!
//! The tint and the dimming are drawn, on every cell of a view of the
//! world. The tint is a coloured filter: each channel of a cell's colours
//! is multiplied by the tint's channel over 255, so that black stays
//! black and white becomes the tint. `dim` then takes that share of the
//! brightness away.
//!
//! Particles are read, checked and kept, and not drawn yet: they come
//! with playback (package U5), which owns the scene's clock.

use serde::Deserialize;
use thiserror::Error;

use crate::color::{ColorRef, Rgb};
use crate::frame::is_single_width;
use crate::theme::Theme;

/// A thousand: the whole, where shares are counted in thousandths.
const WHOLE: u16 = 1000;

/// The particles of an ambience: characters that fall or drift.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticlesDef {
    /// The characters a particle may be drawn as. At least one.
    pub ch: Vec<char>,
    /// The share of cells that hold a particle at any moment, 0 to 1.
    pub density: f64,
    /// Which way they move: `"wind"` for the way the wind blows,
    /// `"down"`, or left out for no drift.
    #[serde(default)]
    pub dir: Option<String>,
    /// How fast they move, in cells per second. Zero or more.
    #[serde(default)]
    pub speed: f64,
}

/// An `ambience` definition (12 §6.4).
///
/// ```json
/// { "type": "ambience", "id": "heavy_rain", "tint": "#8090a0", "dim": 0.2,
///   "particles": { "ch": ["|", "'"], "density": 0.08, "dir": "wind", "speed": 18 } }
/// ```
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmbienceDef {
    /// The colour the whole view is tinted with: a colour name or
    /// `#rrggbb`. Left out, no tint.
    #[serde(default)]
    pub tint: Option<ColorRef>,
    /// The share of brightness taken away, 0 (none) to 1 (black).
    #[serde(default)]
    pub dim: f64,
    /// Particles that fall or drift. Weather particles are drawn only on
    /// cells the character knows to be open to the sky.
    #[serde(default)]
    pub particles: Option<ParticlesDef>,
}

/// Why an ambience cannot be used.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum AmbienceError {
    /// `dim` is not a share.
    #[error("`dim` must be from 0 to 1, and is {0}")]
    DimOutOfRange(f64),
    /// `particles.density` is not a share.
    #[error("`particles.density` must be from 0 to 1, and is {0}")]
    DensityOutOfRange(f64),
    /// `particles.speed` is negative or not a number.
    #[error("`particles.speed` must be zero or more, and is {0}")]
    BadSpeed(f64),
    /// `particles.ch` is empty.
    #[error("`particles.ch` must list at least one character")]
    NoParticleCharacter,
    /// A particle character that does not fill exactly one cell.
    #[error("`{0}` in `particles.ch` is not a single-width character")]
    WideParticle(char),
    /// A tint the theme does not know.
    #[error("unknown colour `{0}` in `tint`")]
    UnknownTint(String),
    /// `material`, `own` or `dress` as a tint.
    #[error("`{0}` in `tint` only means something in a glyph")]
    NotInAGlyph(&'static str),
}

impl AmbienceError {
    /// Where in the definition the mistake is.
    pub fn path(&self) -> &'static str {
        match self {
            AmbienceError::DimOutOfRange(_) => "dim",
            AmbienceError::DensityOutOfRange(_) => "particles.density",
            AmbienceError::BadSpeed(_) => "particles.speed",
            AmbienceError::NoParticleCharacter | AmbienceError::WideParticle(_) => "particles.ch",
            AmbienceError::UnknownTint(_) | AmbienceError::NotInAGlyph(_) => "tint",
        }
    }
}

impl AmbienceDef {
    /// What is wrong with the definition by itself, whatever the theme.
    pub fn problems(&self) -> Vec<AmbienceError> {
        let mut problems = Vec::new();
        let share = |value: f64| (0.0..=1.0).contains(&value);
        if !share(self.dim) {
            problems.push(AmbienceError::DimOutOfRange(self.dim));
        }
        match &self.tint {
            Some(ColorRef::Material) => problems.push(AmbienceError::NotInAGlyph("material")),
            Some(ColorRef::Own) => problems.push(AmbienceError::NotInAGlyph("own")),
            Some(ColorRef::Dress) => problems.push(AmbienceError::NotInAGlyph("dress")),
            Some(ColorRef::Hex(_) | ColorRef::Named(_)) | None => {}
        }
        if let Some(particles) = &self.particles {
            if !share(particles.density) {
                problems.push(AmbienceError::DensityOutOfRange(particles.density));
            }
            if !(particles.speed.is_finite() && particles.speed >= 0.0) {
                problems.push(AmbienceError::BadSpeed(particles.speed));
            }
            if particles.ch.is_empty() {
                problems.push(AmbienceError::NoParticleCharacter);
            }
            if let Some(ch) = particles.ch.iter().find(|ch| !is_single_width(**ch)) {
                problems.push(AmbienceError::WideParticle(*ch));
            }
        }
        problems
    }
}

/// An ambience ready to draw with: its tint resolved through a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Ambience {
    tint: Option<Rgb>,
    /// The share of brightness kept, in thousandths.
    keep: u16,
    particles: Option<ParticlesDef>,
}

impl Ambience {
    /// Checks the definition and resolves its tint in `theme`. Every
    /// problem found is reported.
    pub fn compile(def: &AmbienceDef, theme: &Theme) -> Result<Ambience, Vec<AmbienceError>> {
        let mut problems = def.problems();
        let tint = match &def.tint {
            Some(ColorRef::Hex(rgb)) => Some(*rgb),
            Some(ColorRef::Named(name)) => match theme.color(name) {
                Some(color) => Some(color.rgb),
                None => {
                    problems.push(AmbienceError::UnknownTint(name.clone()));
                    None
                }
            },
            _ => None,
        };
        if !problems.is_empty() {
            return Err(problems);
        }
        let dimmed = (def.dim * f64::from(WHOLE)).round() as u16;
        Ok(Ambience {
            tint,
            keep: WHOLE - dimmed.min(WHOLE),
            particles: def.particles.clone(),
        })
    }

    /// An ambience with a tint and a dimming and no particles, for tests
    /// and tools. `dim` is in thousandths of the brightness.
    pub fn plain(tint: Option<Rgb>, dim: u16) -> Ambience {
        Ambience {
            tint,
            keep: WHOLE - dim.min(WHOLE),
            particles: None,
        }
    }

    /// The colour the view is tinted with.
    pub fn tint(&self) -> Option<Rgb> {
        self.tint
    }

    /// The share of brightness kept, in thousandths.
    pub fn keep(&self) -> u16 {
        self.keep
    }

    /// The particles, as written. Not drawn yet.
    pub fn particles(&self) -> Option<&ParticlesDef> {
        self.particles.as_ref()
    }

    /// A colour as seen through the tint, before dimming: each channel
    /// multiplied by the tint's over 255.
    #[inline]
    pub fn filter(&self, rgb: Rgb) -> Rgb {
        match self.tint {
            Some(tint) => multiply(rgb, tint),
            None => rgb,
        }
    }
}

/// Each channel of `rgb` multiplied by that of `by` over 255, rounded.
#[inline]
pub(crate) const fn multiply(rgb: Rgb, by: Rgb) -> Rgb {
    const fn channel(a: u8, b: u8) -> u8 {
        ((a as u16 * b as u16 + 127) / 255) as u8
    }
    Rgb::new(
        channel(rgb.r, by.r),
        channel(rgb.g, by.g),
        channel(rgb.b, by.b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(json: &str) -> AmbienceDef {
        serde_json::from_str(json).expect("ambience json")
    }

    #[test]
    fn the_ambience_of_the_specification_compiles() {
        let rain = def(r##"{ "tint": "#8090a0", "dim": 0.2,
              "particles": { "ch": ["|", "'"], "density": 0.08, "dir": "wind", "speed": 18 } }"##);
        let ambience = Ambience::compile(&rain, &Theme::default()).unwrap();
        assert_eq!(ambience.tint(), Some(Rgb::new(0x80, 0x90, 0xa0)));
        assert_eq!(ambience.keep(), 800);
        let particles = ambience.particles().unwrap();
        assert_eq!(particles.ch, ['|', '\'']);
        assert_eq!(particles.dir.as_deref(), Some("wind"));
        // The tint is a filter: white becomes the tint, black stays.
        assert_eq!(ambience.filter(Rgb::WHITE), Rgb::new(0x80, 0x90, 0xa0));
        assert_eq!(ambience.filter(Rgb::BLACK), Rgb::BLACK);
        assert_eq!(
            ambience.filter(Rgb::new(100, 200, 50)),
            Rgb::new(50, 113, 31)
        );
    }

    #[test]
    fn an_ambience_may_be_only_a_named_tint_or_only_a_dimming() {
        let dusk = Ambience::compile(&def(r#"{ "tint": "red" }"#), &Theme::default()).unwrap();
        assert_eq!(dusk.tint(), Some(crate::Ansi16::Red.rgb()));
        assert_eq!(dusk.keep(), 1000);
        let dark = Ambience::compile(&def(r#"{ "dim": 1 }"#), &Theme::default()).unwrap();
        assert_eq!((dark.tint(), dark.keep()), (None, 0));
        assert_eq!(dark.filter(Rgb::WHITE), Rgb::WHITE);
    }

    #[test]
    fn every_problem_in_an_ambience_is_reported() {
        let bad = def(r#"{ "tint": "octarine", "dim": 1.5,
              "particles": { "ch": [], "density": 2, "speed": -1 } }"#);
        let problems = Ambience::compile(&bad, &Theme::default()).unwrap_err();
        let paths: Vec<&str> = problems.iter().map(AmbienceError::path).collect();
        assert_eq!(
            paths,
            [
                "dim",
                "particles.density",
                "particles.speed",
                "particles.ch",
                "tint"
            ]
        );
        let own = Ambience::compile(&def(r#"{ "tint": "own" }"#), &Theme::default()).unwrap_err();
        assert_eq!(own, [AmbienceError::NotInAGlyph("own")]);
        let wide = def(r#"{ "particles": { "ch": ["漢"], "density": 0.1 } }"#);
        assert_eq!(wide.problems(), [AmbienceError::WideParticle('漢')]);
        assert!(serde_json::from_str::<AmbienceDef>(r#"{ "tnit": "red" }"#).is_err());
    }
}
