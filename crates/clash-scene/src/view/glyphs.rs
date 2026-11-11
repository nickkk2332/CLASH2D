//! [`WorldGlyphs`]: how every element state is drawn, worked out once.
//!
//! The world's definitions carry a glyph for each state of each floor,
//! block and fixture, and a colour name for each material, all as text
//! (`GlyphSpec`). [`WorldGlyphs::build`] compiles each of them through a
//! [`Theme`] once and keeps the result by layer, definition and state, so
//! that drawing a cell is two indexed reads and no lookup by name.
//!
//! A table is made for one set of definitions and one theme. When either
//! changes, make another.
//!
//! # What is not drawn yet
//!
//! - **Shapes.** A glyph's `shape`, for things of several cells (12 §4.1),
//!   is not part of the world's `GlyphSpec`, so no element has one. An
//!   [`EntitySprite`](super::EntitySprite) carries its facing for when
//!   entity definitions bring shapes.
//! - **Fields and decals.** The world has ids for them and no
//!   definitions yet. [`WorldGlyphs::set_field_look`] and
//!   [`WorldGlyphs::set_decal_tint`] are where their looks are given
//!   once it has; until then a field or a decal in the view model draws
//!   nothing.

use clash_core::DenseId;
use clash_world::defs::{Element, GlyphSpec, Table};
use clash_world::{DecalId, ElementState, FieldId, Layer, MaterialId, WorldDefs};

use crate::color::{Ansi16, Color, ColorRef};
use crate::frame::{Attrs, CellStyle};
use crate::glyph::{Glyph, GlyphDef, GlyphError, GlyphInputs, GlyphPart};
use crate::theme::{Style, Theme};

/// The theme mark drawn where several items lie.
pub const MARK_PILE: &str = "pile";
/// The theme mark drawn for a presence that is suspected and not
/// detected.
pub const MARK_SUSPECTED: &str = "suspected";
/// The theme mark drawn for something only heard.
pub const MARK_HEARD: &str = "heard";
/// The theme mark drawn where a view looks down further than `look_down`
/// storeys and finds nothing: the stratum's backdrop until `sky`
/// definitions supply one.
pub const MARK_BACKDROP: &str = "backdrop";
/// The theme mark whose character a floor is drawn with in a section
/// view, where it is seen edge on.
pub const MARK_SECTION_FLOOR: &str = "section_floor";
/// The theme style the cell under the cursor is drawn in.
pub const STYLE_CURSOR: &str = "cursor";
/// The theme colour the built-in `light` overlay shades with.
pub const COLOR_OVERLAY_LIGHT: &str = "overlay_light";

/// What is wrong with the look of a world definition under a theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphProblem {
    /// The definition type: `"floor"`, `"block"`, `"fixture"` or
    /// `"material"`.
    pub kind: &'static str,
    /// The definition's qualified id.
    pub id: String,
    /// The declared state whose glyph is wrong, if the definition
    /// declares states.
    pub state: Option<String>,
    /// The field that is wrong: `"ch"`, `"fg"`, `"bg"` or `"color"`.
    pub field: &'static str,
    /// What is wrong.
    pub message: String,
}

impl core::fmt::Display for GlyphProblem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} `{}`", self.kind, self.id)?;
        if let Some(state) = &self.state {
            write!(f, ", state `{state}`")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// How a field is drawn (12 §6.1, rule 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLook {
    /// The field's glyph. Its `by_level` entries are indexed by `band`.
    pub glyph: Glyph,
    /// The lowest level at which the glyph is drawn. Below it the field
    /// only tints the background.
    pub glyph_from: u8,
    /// How many levels share one entry of the glyph's `by_level`.
    pub band: u8,
    /// The colour a weaker field tints the background toward.
    pub tint: Option<Color>,
}

/// The glyphs the engine draws for itself in a view of the world, taken
/// from the theme's `marks` with plain defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewMarks {
    /// Several items lying in a cell.
    pub pile: GlyphPart,
    /// A presence that is suspected and not detected.
    pub suspected: GlyphPart,
    /// Something only heard.
    pub heard: GlyphPart,
    /// What lies beyond `look_down` storeys of open cells.
    pub backdrop: GlyphPart,
    /// The character a floor is drawn with in a section view.
    pub section_floor: char,
}

/// The default character of the `pile` mark.
const DEFAULT_PILE: char = '&';
/// The default character of the `suspected` mark.
const DEFAULT_SUSPECTED: char = '?';
/// The default character of the `heard` mark.
const DEFAULT_HEARD: char = '!';
/// The default character of the `backdrop` mark.
const DEFAULT_BACKDROP: char = ' ';
/// The default character of the `section_floor` mark.
const DEFAULT_SECTION_FLOOR: char = '_';

impl ViewMarks {
    fn new(theme: &Theme) -> ViewMarks {
        let mark = |name: &str, ch: char, bold: bool| {
            let fallback = GlyphPart {
                ch: Some(ch),
                bold: bold.then_some(true),
                ..GlyphPart::default()
            };
            match theme.mark(name) {
                Some(glyph) => glyph.part(&GlyphInputs::default()).over(fallback),
                None => fallback,
            }
        };
        ViewMarks {
            pile: mark(MARK_PILE, DEFAULT_PILE, false),
            suspected: mark(MARK_SUSPECTED, DEFAULT_SUSPECTED, true),
            heard: mark(MARK_HEARD, DEFAULT_HEARD, true),
            backdrop: mark(MARK_BACKDROP, DEFAULT_BACKDROP, false),
            section_floor: mark(MARK_SECTION_FLOOR, DEFAULT_SECTION_FLOOR, false)
                .ch
                .unwrap_or(DEFAULT_SECTION_FLOOR),
        }
    }
}

/// The glyph of each state of one element definition. `None` is a state
/// with no character: it is not drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ElementLook {
    states: Vec<Option<GlyphPart>>,
}

/// How every element state, material and mark is drawn under one theme.
/// See the [module](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldGlyphs {
    /// By [`Layer::index`], then by definition id.
    layers: [Vec<ElementLook>; 3],
    /// By material id: the material's colour, if it names one the theme
    /// knows.
    materials: Vec<Option<Color>>,
    /// By field id.
    fields: Vec<Option<FieldLook>>,
    /// By decal id.
    decals: Vec<Option<Color>>,
    marks: ViewMarks,
    base: CellStyle,
    cursor: Style,
    overlay_light: Color,
}

/// Compiles one state's glyph. A glyph with no character is `None`.
fn compile_spec(
    spec: &GlyphSpec,
    theme: &Theme,
    mut problem: impl FnMut(&'static str, String),
) -> Option<GlyphPart> {
    let text = spec.ch.as_deref()?;
    let mut chars = text.chars();
    let ch = match (chars.next(), chars.next()) {
        (Some(ch), None) => ch,
        _ => {
            problem(
                "ch",
                format!("`{text}` is not one character: a glyph's `ch` is exactly one"),
            );
            return None;
        }
    };
    let color = |value: &Option<String>,
                 field: &'static str,
                 problem: &mut dyn FnMut(&'static str, String)| {
        let text = value.as_deref()?;
        match ColorRef::parse(text) {
            Ok(color) => Some(color),
            Err(error) => {
                problem(field, error.to_string());
                None
            }
        }
    };
    let mut def = GlyphDef {
        ch: Some(ch),
        fg: color(&spec.fg, "fg", &mut problem),
        bg: color(&spec.bg, "bg", &mut problem),
        ..GlyphDef::default()
    };
    // Report every mistake in the glyph, dropping the part that is wrong
    // so that the rest still draws.
    loop {
        match def.compile(theme) {
            Ok(glyph) => return Some(glyph.part(&GlyphInputs::default())),
            Err(GlyphError::UnknownColor { at, name }) => {
                let field = if at == "bg" { "bg" } else { "fg" };
                problem(field, format!("unknown colour `{name}`"));
                if field == "bg" {
                    def.bg = None;
                } else {
                    def.fg = None;
                }
            }
            Err(error) => {
                problem("ch", error.to_string());
                return None;
            }
        }
    }
}

/// Compiles the glyph of every state of every definition of one layer,
/// in id order.
fn compile_table<I: DenseId>(
    table: &Table<I, Element>,
    kind: &'static str,
    theme: &Theme,
    problems: &mut Vec<GlyphProblem>,
) -> Vec<ElementLook> {
    table
        .iter()
        .map(|(id, element)| {
            let states = element
                .glyphs
                .iter()
                .enumerate()
                .map(|(state, spec)| {
                    compile_spec(spec, theme, |field, message| {
                        problems.push(GlyphProblem {
                            kind,
                            id: table.name(id).to_owned(),
                            state: element.state_names.get(state).cloned(),
                            field,
                            message,
                        });
                    })
                })
                .collect();
            ElementLook { states }
        })
        .collect()
}

impl WorldGlyphs {
    /// Compiles the look of every element state and material of `world`
    /// through `theme`.
    ///
    /// A table always comes back. Whatever could not be compiled is
    /// listed beside it and draws as plainly as it can: a colour the
    /// theme does not know leaves what is beneath, and a glyph with no
    /// usable character is not drawn. A caller that wants bad content to
    /// stop it treats a non-empty list as an error; [`UiDefs`]
    /// (`world_glyphs`) does, and reports each at the file that wrote it.
    ///
    /// [`UiDefs`]: crate::UiDefs
    pub fn build(world: &WorldDefs, theme: &Theme) -> (WorldGlyphs, Vec<GlyphProblem>) {
        let mut problems = Vec::new();

        let mut materials = Vec::with_capacity(world.materials().len());
        for (id, material) in world.materials().iter() {
            let color = material.color.as_deref().and_then(|name| {
                let found = match ColorRef::parse(name) {
                    Ok(ColorRef::Hex(rgb)) => Some(Color::from_rgb(rgb)),
                    Ok(ColorRef::Named(name)) => theme.color(&name),
                    Ok(_) | Err(_) => None,
                };
                if found.is_none() {
                    problems.push(GlyphProblem {
                        kind: "material",
                        id: world.materials().name(id).to_owned(),
                        state: None,
                        field: "color",
                        message: format!("unknown colour `{name}`"),
                    });
                }
                found
            });
            materials.push(color);
        }

        let layers = [
            compile_table(world.floors(), "floor", theme, &mut problems),
            compile_table(world.blocks(), "block", theme, &mut problems),
            compile_table(world.fixtures(), "fixture", theme, &mut problems),
        ];

        // A problem that several states of a definition share is one
        // problem: the definition's own glyph is wrong. The first state
        // that shows it is kept.
        let mut kept: Vec<GlyphProblem> = Vec::with_capacity(problems.len());
        for problem in problems {
            let shared = kept.iter().any(|earlier| {
                earlier.kind == problem.kind
                    && earlier.id == problem.id
                    && earlier.field == problem.field
                    && earlier.message == problem.message
            });
            if !shared {
                kept.push(problem);
            }
        }
        let problems = kept;

        let glyphs = WorldGlyphs {
            layers,
            materials,
            ..WorldGlyphs::empty(theme)
        };
        (glyphs, problems)
    }

    /// A table with no definitions at all: every element is undrawn.
    /// For a view of a world that has none.
    pub fn empty(theme: &Theme) -> WorldGlyphs {
        WorldGlyphs {
            layers: [Vec::new(), Vec::new(), Vec::new()],
            materials: Vec::new(),
            fields: Vec::new(),
            decals: Vec::new(),
            marks: ViewMarks::new(theme),
            base: theme.base(),
            cursor: theme
                .style(STYLE_CURSOR)
                .unwrap_or(Style::attrs(Attrs::REVERSE)),
            overlay_light: theme
                .color(COLOR_OVERLAY_LIGHT)
                .unwrap_or(Color::ansi(Ansi16::Yellow)),
        }
    }

    /// The glyph of an element state on a layer, or `None` if there is
    /// no element or it has no character. As for the world's own tables,
    /// bits past the last declared state mean the first.
    #[inline]
    pub fn element(&self, layer: Layer, state: ElementState) -> Option<GlyphPart> {
        if state.is_none() {
            return None;
        }
        let look = self.layers[layer.index()].get(usize::from(state.def))?;
        match look.states.get(usize::from(state.bits)) {
            Some(part) => *part,
            None => look.states.first().copied().flatten(),
        }
    }

    /// The colour of a material, for `"material"` in a glyph.
    #[inline]
    pub fn material(&self, material: MaterialId) -> Option<Color> {
        self.materials.get(material.index()).copied().flatten()
    }

    /// How a field is drawn, if its look has been given.
    #[inline]
    pub fn field(&self, field: FieldId) -> Option<&FieldLook> {
        self.fields.get(field.index())?.as_ref()
    }

    /// The colour a decal tints toward, if it has been given.
    #[inline]
    pub fn decal(&self, decal: DecalId) -> Option<Color> {
        self.decals.get(decal.index()).copied().flatten()
    }

    /// Gives a field its look. Called by whoever loads `field`
    /// definitions, once the world has them.
    pub fn set_field_look(&mut self, field: FieldId, look: FieldLook) {
        let index = field.index();
        if self.fields.len() <= index {
            self.fields.resize(index + 1, None);
        }
        self.fields[index] = Some(look);
    }

    /// Gives a decal the colour it tints toward. Called by whoever loads
    /// `decal` definitions, once the world has them.
    pub fn set_decal_tint(&mut self, decal: DecalId, tint: Color) {
        let index = decal.index();
        if self.decals.len() <= index {
            self.decals.resize(index + 1, None);
        }
        self.decals[index] = Some(tint);
    }

    /// The marks a view draws for itself.
    pub fn marks(&self) -> &ViewMarks {
        &self.marks
    }

    /// What an empty cell is drawn in: the theme's text colour on its
    /// background.
    pub fn base(&self) -> CellStyle {
        self.base
    }

    /// The style of the cell under the cursor.
    pub fn cursor(&self) -> Style {
        self.cursor
    }

    /// The colour the built-in `light` overlay shades with.
    pub fn overlay_light(&self) -> Color {
        self.overlay_light
    }
}
