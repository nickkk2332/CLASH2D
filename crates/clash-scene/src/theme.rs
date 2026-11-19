//! Themes: the `theme` definition (12 §4.3) and the [`Theme`] built from
//! it, through which every colour and style name is resolved.
//!
//! # How a name is looked up
//!
//! A colour name is, in order: a colour of the active theme; a `color`
//! definition; one of the sixteen terminal colours (`red`,
//! `bright_blue`, …), which always exist. A theme colour with the same
//! name as a `color` definition overrides its `rgb` and keeps its
//! `fallback` and `word`.
//!
//! A style name is a style of the active theme, or else a colour name,
//! which then colours the text.
//!
//! # Names the engine itself uses
//!
//! The colours `text` and `background` are what an empty cell is drawn
//! in. The interface toolkit draws its own parts with the styles listed
//! in [`crate::ui`]. A theme that leaves any of them out gets a plain
//! default, never an error.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use crate::color::{Ansi16, Color, ColorDef, ColorRef, Rgb};
use crate::frame::{Attrs, Borders, CellStyle, is_single_width};
use crate::glyph::{Glyph, GlyphDef, GlyphError};

/// The theme colour an empty cell's characters are drawn in.
pub const COLOR_TEXT: &str = "text";
/// The theme colour behind everything.
pub const COLOR_BACKGROUND: &str = "background";

/// The text colour of a theme that does not give one.
const DEFAULT_TEXT: Rgb = Rgb::new(0xc8, 0xc8, 0xc8);
/// The background colour of a theme that does not give one.
const DEFAULT_BACKGROUND: Rgb = Rgb::BLACK;

/// Why a theme cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ThemeError {
    /// A colour name that is not a theme colour, a `color` definition or
    /// a terminal colour.
    #[error("unknown colour `{name}` in `{at}`")]
    UnknownColor {
        /// Where in the theme, such as `styles.title.fg`.
        at: String,
        /// The name.
        name: String,
    },
    /// Theme colours that are defined in terms of each other.
    #[error("the colour `{name}` is defined in terms of itself")]
    ColorCycle {
        /// One of the colours in the cycle.
        name: String,
    },
    /// `material`, `own` or `dress` where there is no thing to take the
    /// colour from.
    #[error("`{what}` in `{at}` only means something in a glyph")]
    NotInAGlyph {
        /// Where in the theme.
        at: String,
        /// The word used.
        what: &'static str,
    },
    /// A border character that does not fill exactly one cell.
    #[error("the border character `{ch}` is not a single-width character")]
    WideBorder {
        /// The character.
        ch: char,
    },
    /// A mark whose glyph is wrong.
    #[error("in the mark `{mark}`: {error}")]
    Mark {
        /// The mark's name.
        mark: String,
        /// What is wrong with it.
        error: GlyphError,
    },
}

/// A style in a theme: colours and attributes applied over whatever is
/// beneath. Anything left out is left as it was.
///
/// ```json
/// { "fg": "accent", "bold": true }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleDef {
    /// The colour of the characters: a colour name or `#rrggbb`.
    #[serde(default)]
    pub fg: Option<ColorRef>,
    /// The colour behind them: a colour name or `#rrggbb`.
    #[serde(default)]
    pub bg: Option<ColorRef>,
    /// Bold on (`true`) or off (`false`).
    #[serde(default)]
    pub bold: Option<bool>,
    /// Dim on or off.
    #[serde(default)]
    pub dim: Option<bool>,
    /// Italic on or off.
    #[serde(default)]
    pub italic: Option<bool>,
    /// Underline on or off.
    #[serde(default)]
    pub underline: Option<bool>,
    /// Exchange foreground and background, on or off.
    #[serde(default)]
    pub reverse: Option<bool>,
}

/// The characters borders are drawn with. Any left out keeps the engine's
/// default line-drawing character.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BordersDef {
    /// Horizontal edge.
    #[serde(default)]
    pub h: Option<char>,
    /// Vertical edge.
    #[serde(default)]
    pub v: Option<char>,
    /// Top-left corner.
    #[serde(default)]
    pub tl: Option<char>,
    /// Top-right corner.
    #[serde(default)]
    pub tr: Option<char>,
    /// Bottom-left corner.
    #[serde(default)]
    pub bl: Option<char>,
    /// Bottom-right corner.
    #[serde(default)]
    pub br: Option<char>,
}

/// The font a window frontend draws with (12 §11). The terminal frontend
/// ignores it. Bold, italic and underline are made from this one file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontDef {
    /// The font file (TrueType or OpenType), relative to the directory of
    /// the mod whose file wrote this `font`, so that a mod patching a
    /// theme ships its font in its own directory (decision D-844).
    pub file: String,
    /// The height of a cell in pixels at the screen's ordinary scale; a
    /// screen of scale 2 draws cells twice as high. The window draws
    /// between 4 and 512 pixels high.
    pub size: u16,
}

/// A `theme` definition (12 §4.3).
///
/// ```json
/// { "type": "theme", "id": "default",
///   "colors": { "text": "#c8c8c8", "dim": "#707070", "accent": "#e0b040" },
///   "styles": { "title": { "fg": "accent", "bold": true } },
///   "borders": { "h": "─", "v": "│", "tl": "┌", "tr": "┐", "bl": "└", "br": "┘" },
///   "font": { "file": "assets/font.ttf", "size": 16 } }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeDef {
    /// Colours by name. Each value is `#rrggbb` or the name of another
    /// colour. A name that is also a `color` definition overrides that
    /// definition's `rgb`.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    /// Styles by name, used by widgets' `style` and by inline tags.
    #[serde(default)]
    pub styles: BTreeMap<String, StyleDef>,
    /// The characters borders are drawn with.
    #[serde(default)]
    pub borders: BordersDef,
    /// The font for a window frontend.
    #[serde(default)]
    pub font: Option<FontDef>,
    /// The glyphs the engine draws for itself, by name: the marks of the
    /// world view (a pile of items, a suspected presence, something only
    /// heard, a capsized vessel) and the parts of widgets (see
    /// [`crate::ui`]).
    #[serde(default)]
    pub marks: BTreeMap<String, GlyphDef>,
}

/// A resolved style: what to change in the cell beneath.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    /// The colour of the characters, if the style sets one.
    pub fg: Option<Color>,
    /// The colour behind them, if the style sets one.
    pub bg: Option<Color>,
    /// Attributes the style turns on.
    pub set: Attrs,
    /// Attributes the style turns off.
    pub clear: Attrs,
}

impl Style {
    /// A style that only colours the characters.
    pub const fn fg(color: Color) -> Style {
        Style {
            fg: Some(color),
            bg: None,
            set: Attrs::empty(),
            clear: Attrs::empty(),
        }
    }

    /// A style that only turns attributes on.
    pub const fn attrs(set: Attrs) -> Style {
        Style {
            fg: None,
            bg: None,
            set,
            clear: Attrs::empty(),
        }
    }

    /// The style applied over `under`.
    pub fn over(self, under: CellStyle) -> CellStyle {
        CellStyle {
            fg: self.fg.unwrap_or(under.fg),
            bg: self.bg.unwrap_or(under.bg),
            attrs: (under.attrs - self.clear) | self.set,
        }
    }
}

/// A theme ready to use: every name resolved to colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    colors: BTreeMap<String, Color>,
    words: BTreeMap<String, String>,
    styles: BTreeMap<String, Style>,
    borders: Borders,
    font: Option<FontDef>,
    marks: BTreeMap<String, Glyph>,
    base: CellStyle,
}

impl Default for Theme {
    /// The theme of an engine with no mods: no named colours beyond the
    /// sixteen terminal colours, no styles, line-drawing borders.
    fn default() -> Theme {
        Theme {
            colors: BTreeMap::new(),
            words: BTreeMap::new(),
            styles: BTreeMap::new(),
            borders: Borders::default(),
            font: None,
            marks: BTreeMap::new(),
            base: CellStyle {
                fg: Color::from_rgb(DEFAULT_TEXT),
                bg: Color::from_rgb(DEFAULT_BACKGROUND),
                attrs: Attrs::empty(),
            },
        }
    }
}

impl Theme {
    /// Builds a theme from its definition and the `color` definitions in
    /// force, keyed by id. Every problem found is reported, not only the
    /// first.
    pub fn build(
        def: &ThemeDef,
        colors: &BTreeMap<String, ColorDef>,
    ) -> Result<Theme, Vec<ThemeError>> {
        let mut errors = Vec::new();
        let mut theme = Theme::default();

        for (id, color) in colors {
            theme.colors.insert(id.clone(), color.color());
            theme.words.insert(id.clone(), color.word(id).to_owned());
        }
        for name in def.colors.keys() {
            let mut trail = Vec::new();
            match resolve_theme_color(name, def, colors, &mut trail) {
                Ok(color) => {
                    theme.colors.insert(name.clone(), color);
                }
                Err(error) => {
                    if !errors.contains(&error) {
                        errors.push(error);
                    }
                }
            }
        }
        if let Some(text) = theme.colors.get(COLOR_TEXT) {
            theme.base.fg = *text;
        }
        if let Some(background) = theme.colors.get(COLOR_BACKGROUND) {
            theme.base.bg = *background;
        }

        for (name, style) in &def.styles {
            match theme.compile_style(style, &format!("styles.{name}")) {
                Ok(style) => {
                    theme.styles.insert(name.clone(), style);
                }
                Err(error) => errors.push(error),
            }
        }

        let defaults = Borders::default();
        theme.borders = Borders {
            h: def.borders.h.unwrap_or(defaults.h),
            v: def.borders.v.unwrap_or(defaults.v),
            tl: def.borders.tl.unwrap_or(defaults.tl),
            tr: def.borders.tr.unwrap_or(defaults.tr),
            bl: def.borders.bl.unwrap_or(defaults.bl),
            br: def.borders.br.unwrap_or(defaults.br),
        };
        let b = theme.borders;
        for ch in [b.h, b.v, b.tl, b.tr, b.bl, b.br] {
            if !is_single_width(ch) {
                errors.push(ThemeError::WideBorder { ch });
            }
        }
        theme.font = def.font.clone();

        for (name, glyph) in &def.marks {
            match glyph.compile(&theme) {
                Ok(glyph) => {
                    theme.marks.insert(name.clone(), glyph);
                }
                Err(error) => errors.push(ThemeError::Mark {
                    mark: name.clone(),
                    error,
                }),
            }
        }

        if errors.is_empty() {
            Ok(theme)
        } else {
            Err(errors)
        }
    }

    fn compile_style(&self, def: &StyleDef, at: &str) -> Result<Style, ThemeError> {
        let color = |value: &Option<ColorRef>, field: &str| -> Result<Option<Color>, ThemeError> {
            let at = || format!("{at}.{field}");
            Ok(match value {
                None => None,
                Some(ColorRef::Hex(rgb)) => Some(Color::from_rgb(*rgb)),
                Some(ColorRef::Named(name)) => {
                    Some(self.color(name).ok_or_else(|| ThemeError::UnknownColor {
                        at: at(),
                        name: name.clone(),
                    })?)
                }
                Some(ColorRef::Material) => {
                    return Err(ThemeError::NotInAGlyph {
                        at: at(),
                        what: "material",
                    });
                }
                Some(ColorRef::Own) => {
                    return Err(ThemeError::NotInAGlyph {
                        at: at(),
                        what: "own",
                    });
                }
                Some(ColorRef::Dress) => {
                    return Err(ThemeError::NotInAGlyph {
                        at: at(),
                        what: "dress",
                    });
                }
            })
        };
        let mut style = Style {
            fg: color(&def.fg, "fg")?,
            bg: color(&def.bg, "bg")?,
            ..Style::default()
        };
        for (flag, attr) in [
            (def.bold, Attrs::BOLD),
            (def.dim, Attrs::DIM),
            (def.italic, Attrs::ITALIC),
            (def.underline, Attrs::UNDERLINE),
            (def.reverse, Attrs::REVERSE),
        ] {
            match flag {
                Some(true) => style.set |= attr,
                Some(false) => style.clear |= attr,
                None => {}
            }
        }
        Ok(style)
    }

    /// The colour with a name: a theme colour, a `color` definition or one
    /// of the sixteen terminal colours.
    pub fn color(&self, name: &str) -> Option<Color> {
        self.colors
            .get(name)
            .copied()
            .or_else(|| Ansi16::from_name(name).ok().map(Color::ansi))
    }

    /// The word text uses for a colour (15 §8.4): the `word` of its
    /// `color` definition, or the name of a terminal colour.
    pub fn color_word<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        match self.words.get(name) {
            Some(word) => Some(word),
            None => Ansi16::from_name(name).ok().map(Ansi16::name),
        }
    }

    /// The style with a name: a theme style, or else a colour, which
    /// colours the characters. This is what a widget's `style` and an
    /// inline tag name.
    pub fn style(&self, name: &str) -> Option<Style> {
        self.styles
            .get(name)
            .copied()
            .or_else(|| self.color(name).map(Style::fg))
    }

    /// The names of the theme's styles, for suggestions in messages.
    pub fn style_names(&self) -> impl Iterator<Item = &str> {
        self.styles.keys().map(String::as_str)
    }

    /// What an empty cell is drawn in: the colour `text` on the colour
    /// `background`.
    pub fn base(&self) -> CellStyle {
        self.base
    }

    /// The characters borders are drawn with.
    pub fn borders(&self) -> Borders {
        self.borders
    }

    /// The font for a window frontend, if the theme names one.
    pub fn font(&self) -> Option<&FontDef> {
        self.font.as_ref()
    }

    /// One of the glyphs the engine draws for itself.
    pub fn mark(&self, name: &str) -> Option<&Glyph> {
        self.marks.get(name)
    }
}

fn resolve_theme_color<'a>(
    name: &'a str,
    def: &'a ThemeDef,
    colors: &BTreeMap<String, ColorDef>,
    trail: &mut Vec<&'a str>,
) -> Result<Color, ThemeError> {
    if trail.contains(&name) {
        return Err(ThemeError::ColorCycle {
            name: name.to_owned(),
        });
    }
    let Some(value) = def.colors.get(name) else {
        // Not a theme colour: a `color` definition or a terminal colour.
        return colors
            .get(name)
            .map(ColorDef::color)
            .or_else(|| Ansi16::from_name(name).ok().map(Color::ansi))
            .ok_or_else(|| ThemeError::UnknownColor {
                at: format!("colors.{}", trail.last().copied().unwrap_or(name)),
                name: name.to_owned(),
            });
    };
    if value.starts_with('#') {
        let rgb = Rgb::parse(value).map_err(|_| ThemeError::UnknownColor {
            at: format!("colors.{name}"),
            name: value.clone(),
        })?;
        // Overriding a definition's rgb keeps its fallback.
        return Ok(match colors.get(name).and_then(|c| c.fallback) {
            Some(fallback) => Color::new(rgb, fallback),
            None => Color::from_rgb(rgb),
        });
    }
    if value == name {
        // "red": "red" would otherwise be a cycle; it means the colour
        // below the theme.
        return colors
            .get(name)
            .map(ColorDef::color)
            .or_else(|| Ansi16::from_name(name).ok().map(Color::ansi))
            .ok_or_else(|| ThemeError::ColorCycle {
                name: name.to_owned(),
            });
    }
    trail.push(name);
    let result = resolve_theme_color(value, def, colors, trail);
    trail.pop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(json: &str, colors: &str) -> Result<Theme, Vec<ThemeError>> {
        let def: ThemeDef = serde_json::from_str(json).expect("theme json");
        let colors: BTreeMap<String, ColorDef> = serde_json::from_str(colors).expect("colors json");
        Theme::build(&def, &colors)
    }

    #[test]
    fn the_theme_of_the_specification_loads() {
        let t = theme(
            r##"{ "colors": { "text": "#c8c8c8", "dim": "#707070", "accent": "#e0b040", "blood": "#b02020" },
                 "styles": { "title": { "fg": "accent", "bold": true },
                             "selected": { "fg": "#000000", "bg": "accent" } },
                 "borders": { "h": "─", "v": "│", "tl": "┌", "tr": "┐", "bl": "└", "br": "┘" },
                 "font": { "file": "assets/font.ttf", "size": 16 } }"##,
            "{}",
        )
        .unwrap();
        assert_eq!(t.base().fg.rgb, Rgb::new(0xc8, 0xc8, 0xc8));
        assert_eq!(t.color("blood").unwrap().fallback, Ansi16::Red);
        let title = t.style("title").unwrap();
        assert_eq!(title.fg, t.color("accent"));
        assert_eq!(title.set, Attrs::BOLD);
        let selected = t.style("selected").unwrap().over(t.base());
        assert_eq!(selected.fg.rgb, Rgb::BLACK);
        assert_eq!(Some(selected.bg), t.color("accent"));
        assert_eq!(t.style("blood"), Some(Style::fg(t.color("blood").unwrap())));
        assert_eq!(t.style("nope"), None);
        assert_eq!(t.font().unwrap().size, 16);
    }

    #[test]
    fn a_theme_overrides_rgb_and_keeps_fallback_and_word() {
        let t = theme(
            r##"{ "colors": { "crimson": "#c02030", "danger": "crimson", "warn": "yellow" } }"##,
            r##"{ "crimson": { "rgb": "#a01828", "word": "red", "fallback": "magenta" },
                 "moss": { "rgb": "#507030" } }"##,
        )
        .unwrap();
        let crimson = t.color("crimson").unwrap();
        assert_eq!(
            crimson,
            Color::new(Rgb::new(0xc0, 0x20, 0x30), Ansi16::Magenta)
        );
        assert_eq!(t.color("danger"), Some(crimson));
        assert_eq!(t.color("warn"), Some(Color::ansi(Ansi16::Yellow)));
        assert_eq!(t.color_word("crimson"), Some("red"));
        assert_eq!(t.color_word("moss"), Some("moss"));
        assert_eq!(
            t.color("moss").unwrap().fallback,
            Ansi16::nearest(Rgb::new(0x50, 0x70, 0x30))
        );
        assert_eq!(
            t.color("bright_cyan"),
            Some(Color::ansi(Ansi16::BrightCyan))
        );
    }

    #[test]
    fn every_problem_in_a_theme_is_reported() {
        let errors = theme(
            r##"{ "colors": { "a": "b", "b": "a", "c": "octarine", "d": "#12" },
                 "styles": { "x": { "fg": "nothing" }, "y": { "bg": "own" } },
                 "borders": { "h": "漢" },
                 "marks": { "pile": { "fg": "red" } } }"##,
            "{}",
        )
        .unwrap_err();
        let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
        let has = |needle: &str| text.iter().any(|line| line.contains(needle));
        assert!(has("defined in terms of itself"), "{text:?}");
        assert!(has("unknown colour `octarine`"), "{text:?}");
        assert!(has("unknown colour `#12`"), "{text:?}");
        assert!(has("unknown colour `nothing` in `styles.x.fg`"), "{text:?}");
        assert!(
            has("`own` in `styles.y.bg` only means something in a glyph"),
            "{text:?}"
        );
        assert!(has("border character"), "{text:?}");
        assert!(has("in the mark `pile`"), "{text:?}");
    }

    #[test]
    fn styles_turn_attributes_on_and_off() {
        let t = theme(
            r#"{ "styles": { "quiet": { "bold": false, "dim": true } } }"#,
            "{}",
        )
        .unwrap();
        let under = CellStyle {
            attrs: Attrs::BOLD | Attrs::UNDERLINE,
            ..CellStyle::default()
        };
        let over = t.style("quiet").unwrap().over(under);
        assert_eq!(over.attrs, Attrs::DIM | Attrs::UNDERLINE);
    }

    #[test]
    fn unknown_theme_keys_are_errors() {
        let err = serde_json::from_str::<ThemeDef>(r#"{ "colours": {} }"#).unwrap_err();
        assert!(err.to_string().contains("colours"), "{err}");
    }
}
