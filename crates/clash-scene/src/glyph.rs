//! Glyphs: the `glyph` field every drawable definition carries (12 §4.1),
//! and its resolution to a frame cell.
//!
//! A [`GlyphDef`] is what a mod writes. [`GlyphDef::compile`] checks it and
//! looks its colour names up in a [`Theme`] once, giving a [`Glyph`].
//! [`Glyph::cell`] then turns a glyph and the facts about one thing (its
//! state, its level, the colours it carries) into a [`FrameCell`] with no
//! lookups and no allocation, so it can be called for every cell of every
//! frame.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use crate::color::{Color, ColorRef};
use crate::frame::{Attrs, CellStyle, FrameCell, is_single_width};
use crate::theme::Theme;

/// Why a glyph definition cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GlyphError {
    /// The glyph gives neither a character nor a shape.
    #[error("a glyph needs `ch` or `shape`")]
    NoCharacter,
    /// A character that does not fill exactly one cell.
    #[error("`{ch}` in `{at}` is not a single-width character")]
    NotSingleWidth {
        /// Where in the glyph, such as `by_state.open.ch`.
        at: String,
        /// The character.
        ch: char,
    },
    /// A colour name the theme does not know.
    #[error("unknown colour `{name}` in `{at}`")]
    UnknownColor {
        /// Where in the glyph, such as `fg`.
        at: String,
        /// The name.
        name: String,
    },
    /// A shape with no rows, or with rows of different lengths.
    #[error("the shape for `{facing}` must be rows of equal length, at least one cell")]
    RaggedShape {
        /// The facing whose shape is wrong.
        facing: &'static str,
    },
}

/// The parts of a glyph that a state, a level or a look can replace.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphPartDef {
    /// The character: one Unicode character of single width.
    #[serde(default)]
    pub ch: Option<char>,
    /// The colour of the character.
    #[serde(default)]
    pub fg: Option<ColorRef>,
    /// The colour behind it. `null` or left out: whatever is beneath.
    #[serde(default)]
    pub bg: Option<ColorRef>,
    /// Whether the character is drawn bold.
    #[serde(default)]
    pub bold: Option<bool>,
}

/// A direction a thing of several cells can face.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Facing {
    /// Toward the top of the map.
    North,
    /// Toward the right.
    East,
    /// Toward the bottom.
    South,
    /// Toward the left.
    West,
}

impl Facing {
    /// All four, clockwise from north.
    pub const ALL: [Facing; 4] = [Facing::North, Facing::East, Facing::South, Facing::West];

    /// The name used in data.
    pub const fn name(self) -> &'static str {
        match self {
            Facing::North => "north",
            Facing::East => "east",
            Facing::South => "south",
            Facing::West => "west",
        }
    }

    /// How many quarter-turns clockwise from north.
    const fn turns(self) -> u8 {
        self as u8
    }
}

/// A `glyph` as a definition writes it (12 §4.1).
///
/// ```json
/// { "ch": "#", "fg": "material", "bg": null, "bold": false,
///   "by_state": { "open": { "ch": "'" } },
///   "by_level": [ { "ch": "." }, { "ch": "^", "fg": "orange" } ] }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphDef {
    /// The character: one Unicode character of single width. Required
    /// unless `shape` is given.
    #[serde(default)]
    pub ch: Option<char>,
    /// The colour of the character: a colour name, `#rrggbb`, `"material"`,
    /// `"own"` or `"dress"`. Left out, the theme's text colour.
    #[serde(default)]
    pub fg: Option<ColorRef>,
    /// The colour behind the character, written like `fg`. `null` or left
    /// out: whatever is beneath.
    #[serde(default)]
    pub bg: Option<ColorRef>,
    /// Whether the character is drawn bold.
    #[serde(default)]
    pub bold: Option<bool>,
    /// Parts replaced while the element is in a named state.
    #[serde(default)]
    pub by_state: BTreeMap<String, GlyphPartDef>,
    /// Parts replaced by field or damage level: entry 0 for level 0, and
    /// so on. A level past the end uses the last entry.
    #[serde(default)]
    pub by_level: Vec<GlyphPartDef>,
    /// For a thing that fills several cells: rows of characters, one per
    /// cell of its footprint, for a facing. Facings not given are turned
    /// from the first one that is.
    #[serde(default)]
    pub shape: BTreeMap<Facing, Vec<String>>,
    /// Drawn over (positive) or under (negative) its usual place in the
    /// order of 12 §6.1.
    #[serde(default)]
    pub layer: i8,
}

/// Where a glyph's colour comes from, once names are resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paint {
    /// A colour fixed by the definition.
    Fixed(Color),
    /// The colour of the thing's material.
    Material,
    /// The colour this particular thing carries.
    Own,
    /// The colour of what a creature is seen to be wearing.
    Dress,
}

/// Resolved parts of a glyph. `None` leaves what is beneath unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GlyphPart {
    /// The character.
    pub ch: Option<char>,
    /// The colour of the character.
    pub fg: Option<Paint>,
    /// The colour behind it.
    pub bg: Option<Paint>,
    /// Bold or not.
    pub bold: Option<bool>,
}

impl GlyphPart {
    fn compile(def: &GlyphPartDef, theme: &Theme, at: &str) -> Result<GlyphPart, GlyphError> {
        if let Some(ch) = def.ch
            && !is_single_width(ch)
        {
            return Err(GlyphError::NotSingleWidth {
                at: format!("{at}ch"),
                ch,
            });
        }
        let paint = |color: &Option<ColorRef>, field: &str| -> Result<Option<Paint>, GlyphError> {
            Ok(match color {
                None => None,
                Some(ColorRef::Material) => Some(Paint::Material),
                Some(ColorRef::Own) => Some(Paint::Own),
                Some(ColorRef::Dress) => Some(Paint::Dress),
                Some(ColorRef::Hex(rgb)) => Some(Paint::Fixed(Color::from_rgb(*rgb))),
                Some(ColorRef::Named(name)) => {
                    Some(Paint::Fixed(theme.color(name).ok_or_else(|| {
                        GlyphError::UnknownColor {
                            at: format!("{at}{field}"),
                            name: name.clone(),
                        }
                    })?))
                }
            })
        };
        Ok(GlyphPart {
            ch: def.ch,
            fg: paint(&def.fg, "fg")?,
            bg: paint(&def.bg, "bg")?,
            bold: def.bold,
        })
    }

    /// These parts laid over `under`: whatever is given here wins.
    pub fn over(self, under: GlyphPart) -> GlyphPart {
        GlyphPart {
            ch: self.ch.or(under.ch),
            fg: self.fg.or(under.fg),
            bg: self.bg.or(under.bg),
            bold: self.bold.or(under.bold),
        }
    }
}

/// What is known about the one thing being drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GlyphInputs<'a> {
    /// The element's state, for `by_state`.
    pub state: Option<&'a str>,
    /// The field or damage level, for `by_level`.
    pub level: Option<usize>,
    /// The colour of the thing's material, for `"material"`.
    pub material: Option<Color>,
    /// The colour this particular thing carries, for `"own"`.
    pub own: Option<Color>,
    /// The colour of what the creature is seen to be wearing, for
    /// `"dress"`.
    pub dress: Option<Color>,
}

/// A glyph ready to draw: checked, with its colour names resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    base: GlyphPart,
    by_state: Vec<(String, GlyphPart)>,
    by_level: Vec<GlyphPart>,
    shapes: Vec<(Facing, Vec<Vec<char>>)>,
    layer: i8,
}

impl GlyphDef {
    /// Checks the definition and resolves its colour names in `theme`.
    pub fn compile(&self, theme: &Theme) -> Result<Glyph, GlyphError> {
        if self.ch.is_none() && self.shape.is_empty() {
            return Err(GlyphError::NoCharacter);
        }
        let base = GlyphPart::compile(
            &GlyphPartDef {
                ch: self.ch,
                fg: self.fg.clone(),
                bg: self.bg.clone(),
                bold: self.bold,
            },
            theme,
            "",
        )?;
        let mut by_state = Vec::with_capacity(self.by_state.len());
        for (state, part) in &self.by_state {
            let at = format!("by_state.{state}.");
            by_state.push((state.clone(), GlyphPart::compile(part, theme, &at)?));
        }
        let mut by_level = Vec::with_capacity(self.by_level.len());
        for (level, part) in self.by_level.iter().enumerate() {
            let at = format!("by_level[{level}].");
            by_level.push(GlyphPart::compile(part, theme, &at)?);
        }
        let mut shapes = Vec::with_capacity(self.shape.len());
        for (facing, rows) in &self.shape {
            let grid: Vec<Vec<char>> = rows.iter().map(|row| row.chars().collect()).collect();
            let width = grid.first().map_or(0, Vec::len);
            if width == 0 || grid.iter().any(|row| row.len() != width) {
                return Err(GlyphError::RaggedShape {
                    facing: facing.name(),
                });
            }
            if let Some(ch) = grid.iter().flatten().find(|ch| !is_single_width(**ch)) {
                return Err(GlyphError::NotSingleWidth {
                    at: format!("shape.{}", facing.name()),
                    ch: *ch,
                });
            }
            shapes.push((*facing, grid));
        }
        Ok(Glyph {
            base,
            by_state,
            by_level,
            shapes,
            layer: self.layer,
        })
    }
}

impl Glyph {
    /// The glyph's parts for a thing in this state and at this level:
    /// the base, then its state's parts, then its level's.
    pub fn part(&self, inputs: &GlyphInputs<'_>) -> GlyphPart {
        let mut part = self.base;
        if let Some(state) = inputs.state
            && let Some((_, by_state)) = self.by_state.iter().find(|(name, _)| name == state)
        {
            part = by_state.over(part);
        }
        if let Some(level) = inputs.level
            && let Some(by_level) = self.by_level.get(level).or(self.by_level.last())
        {
            part = by_level.over(part);
        }
        part
    }

    /// The cell this glyph draws over `under`. Colours the thing does not
    /// carry (`"own"` on something with no colour of its own) leave what
    /// is beneath.
    pub fn cell(&self, inputs: &GlyphInputs<'_>, under: CellStyle) -> FrameCell {
        resolve_part(self.part(inputs), inputs, under)
    }

    /// Where the glyph is drawn relative to its usual place.
    pub fn layer(&self) -> i8 {
        self.layer
    }

    /// Whether the glyph has a shape for things of several cells.
    pub fn has_shape(&self) -> bool {
        !self.shapes.is_empty()
    }

    /// The size, in columns and rows, of the shape for a facing.
    pub fn shape_size(&self, facing: Facing) -> Option<(usize, usize)> {
        let (given, grid) = self.shape_source(facing)?;
        let (w, h) = (grid[0].len(), grid.len());
        let quarter_turns = (4 + facing.turns() - given.turns()) % 4;
        Some(if quarter_turns % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        })
    }

    /// The character at a column and row of the shape for a facing. A
    /// facing the definition does not give is turned from the first one it
    /// does give.
    pub fn shape_char(&self, facing: Facing, col: usize, row: usize) -> Option<char> {
        let (given, grid) = self.shape_source(facing)?;
        let (w, h) = (grid[0].len(), grid.len());
        // Map the asked-for cell back through the turn to the given grid.
        let (c, r) = match (4 + facing.turns() - given.turns()) % 4 {
            0 => (col, row),
            1 => (row, h.checked_sub(1 + col)?),
            2 => (w.checked_sub(1 + col)?, h.checked_sub(1 + row)?),
            _ => (w.checked_sub(1 + row)?, col),
        };
        grid.get(r)?.get(c).copied()
    }

    fn shape_source(&self, facing: Facing) -> Option<(Facing, &Vec<Vec<char>>)> {
        self.shapes
            .iter()
            .find(|(given, _)| *given == facing)
            .or(self.shapes.first())
            .map(|(given, grid)| (*given, grid))
    }
}

/// Turns glyph parts and one thing's colours into a cell over `under`.
pub fn resolve_part(part: GlyphPart, inputs: &GlyphInputs<'_>, under: CellStyle) -> FrameCell {
    let pick = |paint: Option<Paint>, beneath: Color| match paint {
        None => beneath,
        Some(Paint::Fixed(color)) => color,
        Some(Paint::Material) => inputs.material.unwrap_or(beneath),
        Some(Paint::Own) => inputs.own.unwrap_or(beneath),
        Some(Paint::Dress) => inputs.dress.unwrap_or(beneath),
    };
    let mut attrs = under.attrs;
    match part.bold {
        Some(true) => attrs |= Attrs::BOLD,
        Some(false) => attrs -= Attrs::BOLD,
        None => {}
    }
    FrameCell {
        ch: part.ch.unwrap_or(' '),
        fg: pick(part.fg, under.fg),
        bg: pick(part.bg, under.bg),
        attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Ansi16, Rgb};

    fn glyph(json: &str) -> Result<Glyph, GlyphError> {
        let def: GlyphDef = serde_json::from_str(json).expect("glyph json");
        def.compile(&Theme::default())
    }

    #[test]
    fn states_and_levels_replace_parts() {
        let g = glyph(
            r##"{ "ch": "+", "fg": "yellow", "bold": true,
                 "by_state": { "open": { "ch": "'" } },
                 "by_level": [ { "ch": "." }, { "ch": "^", "fg": "#ff8000" }, { "fg": "red" } ] }"##,
        )
        .unwrap();
        let under = CellStyle::default();
        let closed = g.cell(&GlyphInputs::default(), under);
        assert_eq!(closed.ch, '+');
        assert_eq!(closed.fg, Color::ansi(Ansi16::Yellow));
        assert_eq!(closed.bg, under.bg);
        assert!(closed.attrs.contains(Attrs::BOLD));

        let open = g.cell(
            &GlyphInputs {
                state: Some("open"),
                ..Default::default()
            },
            under,
        );
        assert_eq!(open.ch, '\'');
        let unknown = g.cell(
            &GlyphInputs {
                state: Some("ajar"),
                ..Default::default()
            },
            under,
        );
        assert_eq!(unknown.ch, '+');

        let level = |n| {
            g.cell(
                &GlyphInputs {
                    level: Some(n),
                    ..Default::default()
                },
                under,
            )
        };
        assert_eq!(level(0).ch, '.');
        assert_eq!(level(1).fg.rgb, Rgb::new(0xff, 0x80, 0));
        assert_eq!((level(2).ch, level(2).fg), ('+', Color::ansi(Ansi16::Red)));
        assert_eq!(
            level(99),
            level(2),
            "a level past the end uses the last entry"
        );
    }

    #[test]
    fn own_dress_and_material_take_the_colours_the_thing_carries() {
        let g = glyph(r#"{ "ch": "@", "fg": "dress", "bg": "own" }"#).unwrap();
        let m = glyph(r##"{ "ch": "#", "fg": "material" }"##).unwrap();
        let under = CellStyle::default();
        let red = Color::ansi(Ansi16::Red);
        let blue = Color::ansi(Ansi16::Blue);
        let cell = g.cell(
            &GlyphInputs {
                own: Some(red),
                dress: Some(blue),
                ..Default::default()
            },
            under,
        );
        assert_eq!((cell.fg, cell.bg), (blue, red));
        let bare = g.cell(&GlyphInputs::default(), under);
        assert_eq!((bare.fg, bare.bg), (under.fg, under.bg));
        let wall = m.cell(
            &GlyphInputs {
                material: Some(red),
                ..Default::default()
            },
            under,
        );
        assert_eq!(wall.fg, red);
    }

    #[test]
    fn a_shape_is_turned_for_facings_not_given() {
        let g = glyph(r#"{ "fg": "own", "shape": { "north": ["ab", "cd", "ef"] } }"#).unwrap();
        let grid = |facing: Facing| -> Vec<String> {
            let (w, h) = g.shape_size(facing).unwrap();
            (0..h)
                .map(|r| {
                    (0..w)
                        .map(|c| g.shape_char(facing, c, r).unwrap())
                        .collect()
                })
                .collect()
        };
        assert_eq!(grid(Facing::North), ["ab", "cd", "ef"]);
        assert_eq!(grid(Facing::East), ["eca", "fdb"]);
        assert_eq!(grid(Facing::South), ["fe", "dc", "ba"]);
        assert_eq!(grid(Facing::West), ["bdf", "ace"]);
        assert_eq!(g.shape_char(Facing::East, 3, 0), None);
    }

    #[test]
    fn bad_glyphs_are_errors() {
        assert_eq!(glyph(r#"{ "fg": "red" }"#), Err(GlyphError::NoCharacter));
        assert!(matches!(
            glyph(r#"{ "ch": "漢" }"#),
            Err(GlyphError::NotSingleWidth { .. })
        ));
        assert_eq!(
            glyph(r#"{ "ch": "x", "by_state": { "lit": { "fg": "octarine" } } }"#),
            Err(GlyphError::UnknownColor {
                at: "by_state.lit.fg".into(),
                name: "octarine".into()
            })
        );
        assert_eq!(
            glyph(r#"{ "shape": { "east": ["ab", "c"] } }"#),
            Err(GlyphError::RaggedShape { facing: "east" })
        );
        assert!(serde_json::from_str::<GlyphDef>(r#"{ "ch": "ab" }"#).is_err());
        assert!(serde_json::from_str::<GlyphDef>(r#"{ "ch": "a", "colour": "red" }"#).is_err());
        assert!(serde_json::from_str::<GlyphDef>(r##"{ "ch": "a", "fg": "#12" }"##).is_err());
    }
}
