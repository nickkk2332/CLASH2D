//! Colours: [`Rgb`], the sixteen terminal colours ([`Ansi16`]), the pair a
//! frame cell carries ([`Color`]), and the `color` definition
//! ([`ColorDef`]) of `docs/12-presentation-and-ui.md` section 4.3.
//!
//! A colour that reaches a frame is always a [`Color`]: the exact `rgb` a
//! window can show, and the `fallback` a terminal with sixteen colours
//! shows instead. A frontend chooses between them by its colour depth and
//! never has to look anything up.

use core::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a text is not a colour.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ColorError {
    /// The text did not have the form `#rrggbb`.
    #[error("`{0}` is not a colour: write `#rrggbb` with six hexadecimal digits")]
    NotHex(String),
    /// The name is none of the sixteen terminal colours.
    #[error("`{0}` is not one of the sixteen terminal colours")]
    NotAnsi(String),
}

/// A 24-bit colour.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Rgb {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
}

impl Rgb {
    /// Black.
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    /// White.
    pub const WHITE: Rgb = Rgb::new(255, 255, 255);

    /// A colour from its three parts.
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    /// Reads `#rrggbb`.
    ///
    /// ```
    /// use clash_scene::Rgb;
    /// assert_eq!(Rgb::parse("#a01828"), Ok(Rgb::new(0xa0, 0x18, 0x28)));
    /// assert!(Rgb::parse("red").is_err());
    /// ```
    pub fn parse(text: &str) -> Result<Rgb, ColorError> {
        let bad = || ColorError::NotHex(text.to_owned());
        let digits = text.strip_prefix('#').ok_or_else(bad)?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let part = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|_| bad());
        Ok(Rgb::new(part(0)?, part(2)?, part(4)?))
    }

    /// How far apart two colours look, as a number that only means
    /// something compared with another such number. Integer arithmetic
    /// only, so the nearest colour is the same on every machine.
    pub const fn distance(self, other: Rgb) -> u32 {
        // The "redmean" approximation: weights red and blue by how much red
        // the pair has, which follows the eye better than plain distance.
        let rmean = (self.r as i32 + other.r as i32) / 2;
        let dr = self.r as i32 - other.r as i32;
        let dg = self.g as i32 - other.g as i32;
        let db = self.b as i32 - other.b as i32;
        let d = (((512 + rmean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rmean) * db * db) >> 8);
        d as u32
    }

    /// This colour moved toward `other` by `permille` thousandths.
    pub const fn toward(self, other: Rgb, permille: u16) -> Rgb {
        const fn mix(a: u8, b: u8, permille: u16) -> u8 {
            let p = if permille > 1000 {
                1000
            } else {
                permille as i32
            };
            ((a as i32 * (1000 - p) + b as i32 * p + 500) / 1000) as u8
        }
        Rgb::new(
            mix(self.r, other.r, permille),
            mix(self.g, other.g, permille),
            mix(self.b, other.b, permille),
        )
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Rgb, D::Error> {
        struct RgbVisitor;
        impl Visitor<'_> for RgbVisitor {
            type Value = Rgb;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour written `#rrggbb`")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Rgb, E> {
                Rgb::parse(text).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(RgbVisitor)
    }
}

/// One of the sixteen colours every terminal has.
///
/// In data they are written `black`, `red`, `green`, `yellow`, `blue`,
/// `magenta`, `cyan`, `white`, and each with `bright_` in front.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ansi16 {
    /// Colour 0.
    #[default]
    Black,
    /// Colour 1.
    Red,
    /// Colour 2.
    Green,
    /// Colour 3.
    Yellow,
    /// Colour 4.
    Blue,
    /// Colour 5.
    Magenta,
    /// Colour 6.
    Cyan,
    /// Colour 7: light grey.
    White,
    /// Colour 8: dark grey.
    BrightBlack,
    /// Colour 9.
    BrightRed,
    /// Colour 10.
    BrightGreen,
    /// Colour 11.
    BrightYellow,
    /// Colour 12.
    BrightBlue,
    /// Colour 13.
    BrightMagenta,
    /// Colour 14.
    BrightCyan,
    /// Colour 15.
    BrightWhite,
}

impl Ansi16 {
    /// All sixteen, in terminal order.
    pub const ALL: [Ansi16; 16] = [
        Ansi16::Black,
        Ansi16::Red,
        Ansi16::Green,
        Ansi16::Yellow,
        Ansi16::Blue,
        Ansi16::Magenta,
        Ansi16::Cyan,
        Ansi16::White,
        Ansi16::BrightBlack,
        Ansi16::BrightRed,
        Ansi16::BrightGreen,
        Ansi16::BrightYellow,
        Ansi16::BrightBlue,
        Ansi16::BrightMagenta,
        Ansi16::BrightCyan,
        Ansi16::BrightWhite,
    ];

    /// The colour's number in a terminal, 0 to 15.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// The name used in data.
    pub const fn name(self) -> &'static str {
        match self {
            Ansi16::Black => "black",
            Ansi16::Red => "red",
            Ansi16::Green => "green",
            Ansi16::Yellow => "yellow",
            Ansi16::Blue => "blue",
            Ansi16::Magenta => "magenta",
            Ansi16::Cyan => "cyan",
            Ansi16::White => "white",
            Ansi16::BrightBlack => "bright_black",
            Ansi16::BrightRed => "bright_red",
            Ansi16::BrightGreen => "bright_green",
            Ansi16::BrightYellow => "bright_yellow",
            Ansi16::BrightBlue => "bright_blue",
            Ansi16::BrightMagenta => "bright_magenta",
            Ansi16::BrightCyan => "bright_cyan",
            Ansi16::BrightWhite => "bright_white",
        }
    }

    /// The colour with that name in data, if it is one of the sixteen.
    pub fn from_name(name: &str) -> Result<Ansi16, ColorError> {
        Ansi16::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .ok_or_else(|| ColorError::NotAnsi(name.to_owned()))
    }

    /// The colour most terminals show for it. A terminal's own palette may
    /// differ; this is what "nearest" is measured against.
    pub const fn rgb(self) -> Rgb {
        match self {
            Ansi16::Black => Rgb::new(0, 0, 0),
            Ansi16::Red => Rgb::new(205, 0, 0),
            Ansi16::Green => Rgb::new(0, 205, 0),
            Ansi16::Yellow => Rgb::new(205, 205, 0),
            Ansi16::Blue => Rgb::new(0, 0, 238),
            Ansi16::Magenta => Rgb::new(205, 0, 205),
            Ansi16::Cyan => Rgb::new(0, 205, 205),
            Ansi16::White => Rgb::new(229, 229, 229),
            Ansi16::BrightBlack => Rgb::new(127, 127, 127),
            Ansi16::BrightRed => Rgb::new(255, 0, 0),
            Ansi16::BrightGreen => Rgb::new(0, 255, 0),
            Ansi16::BrightYellow => Rgb::new(255, 255, 0),
            Ansi16::BrightBlue => Rgb::new(92, 92, 255),
            Ansi16::BrightMagenta => Rgb::new(255, 0, 255),
            Ansi16::BrightCyan => Rgb::new(0, 255, 255),
            Ansi16::BrightWhite => Rgb::new(255, 255, 255),
        }
    }

    /// The one of the sixteen nearest to `rgb`. Of two equally near, the
    /// lower number wins.
    pub fn nearest(rgb: Rgb) -> Ansi16 {
        let mut best = Ansi16::Black;
        let mut best_distance = u32::MAX;
        for candidate in Ansi16::ALL {
            let distance = rgb.distance(candidate.rgb());
            if distance < best_distance {
                best = candidate;
                best_distance = distance;
            }
        }
        best
    }
}

impl fmt::Display for Ansi16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The six levels of each channel in the 256-colour cube.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
/// The first colour number of the 6 × 6 × 6 cube.
const CUBE_BASE: u8 = 16;
/// The first colour number of the grey ramp.
const GREY_BASE: u8 = 232;
/// How many greys the ramp has.
const GREY_STEPS: u8 = 24;

/// The colour number, 16 to 255, of the 256-colour palette nearest to
/// `rgb`. Numbers 0 to 15 are never returned, because a terminal's user
/// may have set them to anything.
pub fn ansi256(rgb: Rgb) -> u8 {
    fn level(value: u8) -> u8 {
        let mut best = 0u8;
        let mut best_distance = u16::MAX;
        for (index, level) in CUBE_LEVELS.iter().enumerate() {
            let distance = (i16::from(value) - i16::from(*level)).unsigned_abs();
            if distance < best_distance {
                best = index as u8;
                best_distance = distance;
            }
        }
        best
    }
    let (r, g, b) = (level(rgb.r), level(rgb.g), level(rgb.b));
    let cube = CUBE_BASE + 36 * r + 6 * g + b;
    let cube_rgb = Rgb::new(
        CUBE_LEVELS[r as usize],
        CUBE_LEVELS[g as usize],
        CUBE_LEVELS[b as usize],
    );
    let mean = (u16::from(rgb.r) + u16::from(rgb.g) + u16::from(rgb.b)) / 3;
    let step = (mean.saturating_sub(3) / 10).min(u16::from(GREY_STEPS) - 1) as u8;
    let grey_value = 8 + 10 * step;
    let grey_rgb = Rgb::new(grey_value, grey_value, grey_value);
    if rgb.distance(grey_rgb) < rgb.distance(cube_rgb) {
        GREY_BASE + step
    } else {
        cube
    }
}

/// The colour a frame cell carries: what to show where 24-bit colour is
/// available, and what to show where only sixteen colours are.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Color {
    /// The exact colour.
    pub rgb: Rgb,
    /// The terminal colour used when `rgb` cannot be shown.
    pub fallback: Ansi16,
}

impl Color {
    /// A colour with a stated fallback, as a `color` definition gives.
    pub const fn new(rgb: Rgb, fallback: Ansi16) -> Color {
        Color { rgb, fallback }
    }

    /// A colour written as a bare `#rrggbb`: its fallback is the nearest of
    /// the sixteen (12 §4.3).
    pub fn from_rgb(rgb: Rgb) -> Color {
        Color {
            rgb,
            fallback: Ansi16::nearest(rgb),
        }
    }

    /// One of the sixteen terminal colours as itself.
    pub const fn ansi(color: Ansi16) -> Color {
        Color {
            rgb: color.rgb(),
            fallback: color,
        }
    }
}

/// A `color` definition (12 §4.3).
///
/// ```json
/// { "type": "color", "id": "crimson", "rgb": "#a01828", "word": "red", "fallback": "red" }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorDef {
    /// The colour, as `#rrggbb`. A theme may override it.
    pub rgb: Rgb,
    /// How text names the colour, so that what a witness describes and
    /// what is drawn agree (15 §8.4). Left out, it is the definition's id.
    #[serde(default)]
    pub word: Option<String>,
    /// The terminal colour used when the frontend cannot show `rgb`. Left
    /// out, it is the nearest of the sixteen.
    #[serde(default)]
    pub fallback: Option<Ansi16>,
}

impl ColorDef {
    /// The colour a frame carries for this definition.
    pub fn color(&self) -> Color {
        Color {
            rgb: self.rgb,
            fallback: self.fallback.unwrap_or_else(|| Ansi16::nearest(self.rgb)),
        }
    }

    /// The word text uses for the colour, given the definition's id.
    pub fn word<'a>(&'a self, id: &'a str) -> &'a str {
        match &self.word {
            Some(word) => word,
            None => id.rsplit(':').next().unwrap_or(id),
        }
    }
}

/// How a definition names a colour: the value of a glyph's or a style's
/// `fg` and `bg` (12 §4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorRef {
    /// A bare `#rrggbb`.
    Hex(Rgb),
    /// `"material"`: the colour of the thing's material.
    Material,
    /// `"own"`: the colour this particular thing carries (06 §9.1).
    Own,
    /// `"dress"`: the colour of what a creature is seen to be wearing.
    Dress,
    /// The name of a theme colour, a `color` definition or one of the
    /// sixteen terminal colours.
    Named(String),
}

impl ColorRef {
    /// Reads the value of an `fg` or `bg` field.
    pub fn parse(text: &str) -> Result<ColorRef, ColorError> {
        Ok(match text {
            "material" => ColorRef::Material,
            "own" => ColorRef::Own,
            "dress" => ColorRef::Dress,
            hex if hex.starts_with('#') => ColorRef::Hex(Rgb::parse(hex)?),
            name => ColorRef::Named(name.to_owned()),
        })
    }
}

impl<'de> Deserialize<'de> for ColorRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ColorRef, D::Error> {
        struct RefVisitor;
        impl Visitor<'_> for RefVisitor {
            type Value = ColorRef;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour name, `#rrggbb`, `material`, `own` or `dress`")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<ColorRef, E> {
                ColorRef::parse(text).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(RefVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_what_is_not_hex() {
        let c = Rgb::parse("#0aFf10").unwrap();
        assert_eq!(c, Rgb::new(0x0a, 0xff, 0x10));
        assert_eq!(c.to_string(), "#0aff10");
        for bad in ["", "#", "#12345", "#1234567", "123456", "#12345g", "#ééé"] {
            assert_eq!(Rgb::parse(bad), Err(ColorError::NotHex(bad.to_owned())));
        }
    }

    #[test]
    fn every_terminal_colour_is_its_own_nearest() {
        for c in Ansi16::ALL {
            assert_eq!(Ansi16::nearest(c.rgb()), c);
            assert_eq!(Ansi16::from_name(c.name()), Ok(c));
            assert_eq!(Ansi16::ALL[c.index() as usize], c);
        }
    }

    #[test]
    fn a_bare_hex_maps_to_the_nearest_of_sixteen() {
        assert_eq!(Ansi16::nearest(Rgb::new(0xa0, 0x18, 0x28)), Ansi16::Red);
        assert_eq!(Ansi16::nearest(Rgb::new(0x10, 0x10, 0x10)), Ansi16::Black);
        assert_eq!(
            Ansi16::nearest(Rgb::new(0xf8, 0xf8, 0xf0)),
            Ansi16::BrightWhite
        );
        assert_eq!(
            Ansi16::nearest(Rgb::new(0x70, 0x70, 0x70)),
            Ansi16::BrightBlack
        );
        assert_eq!(Ansi16::nearest(Rgb::new(0x20, 0xc0, 0x30)), Ansi16::Green);
    }

    #[test]
    fn the_256_palette_uses_the_cube_and_the_grey_ramp() {
        assert_eq!(ansi256(Rgb::new(0, 0, 0)), 16);
        assert_eq!(ansi256(Rgb::new(255, 255, 255)), 231);
        assert_eq!(ansi256(Rgb::new(255, 0, 0)), 196);
        assert_eq!(ansi256(Rgb::new(0x80, 0x80, 0x80)), 244);
        assert_eq!(ansi256(Rgb::new(0x12, 0x12, 0x12)), 233);
        for value in 0..=255u8 {
            assert!(ansi256(Rgb::new(value, value / 2, 255 - value)) >= 16);
        }
    }

    #[test]
    fn a_color_definition_loads_with_word_and_fallback() {
        let def: ColorDef =
            serde_json::from_str(r##"{ "rgb": "#a01828", "word": "red", "fallback": "red" }"##)
                .unwrap();
        assert_eq!(
            def.color(),
            Color::new(Rgb::new(0xa0, 0x18, 0x28), Ansi16::Red)
        );
        assert_eq!(def.word("ui:crimson"), "red");

        let bare: ColorDef = serde_json::from_str(r##"{ "rgb": "#1020c0" }"##).unwrap();
        assert_eq!(bare.color().fallback, Ansi16::Blue);
        assert_eq!(bare.word("ui:royal"), "royal");
    }

    #[test]
    fn bad_colour_data_is_an_error_not_a_panic() {
        let err = serde_json::from_str::<ColorDef>(r##"{ "rgb": "#a0182" }"##).unwrap_err();
        assert!(err.to_string().contains("is not a colour"), "{err}");
        let err = serde_json::from_str::<ColorDef>(r##"{ "rgb": "#a01828", "fallback": "pink" }"##)
            .unwrap_err();
        assert!(err.to_string().contains("pink"), "{err}");
        let err = serde_json::from_str::<ColorDef>(r##"{ "rgb": "#a01828", "wrod": "red" }"##)
            .unwrap_err();
        assert!(err.to_string().contains("wrod"), "{err}");
    }

    #[test]
    fn colour_references_are_told_apart() {
        assert_eq!(ColorRef::parse("own"), Ok(ColorRef::Own));
        assert_eq!(ColorRef::parse("dress"), Ok(ColorRef::Dress));
        assert_eq!(ColorRef::parse("material"), Ok(ColorRef::Material));
        assert_eq!(ColorRef::parse("#000000"), Ok(ColorRef::Hex(Rgb::BLACK)));
        assert_eq!(
            ColorRef::parse("accent"),
            Ok(ColorRef::Named("accent".into()))
        );
        assert!(ColorRef::parse("#nothex").is_err());
    }

    #[test]
    fn mixing_moves_toward_the_other_colour() {
        let a = Rgb::new(0, 100, 200);
        assert_eq!(a.toward(Rgb::WHITE, 0), a);
        assert_eq!(a.toward(Rgb::WHITE, 1000), Rgb::WHITE);
        assert_eq!(a.toward(Rgb::BLACK, 500), Rgb::new(0, 50, 100));
    }
}
