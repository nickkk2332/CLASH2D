//! Overlays: what a `map_view` may draw on top of its cells (12 §6.2).
//!
//! An [`OverlayDef`] is what a mod writes: an expression over a cell and
//! a colour to shade the cells where it holds. [`Overlay::compile`]
//! resolves the colour through a theme and keeps the expression as its
//! source: evaluating it is the binding package's (U6b), which supplies
//! an [`OverlayValues`].
//!
//! A `map_view` names its overlay in its `overlay` field: the qualified
//! id of an `overlay` definition, or the name of one the engine draws
//! itself ([`BuiltinOverlay`]).
//!
//! # An overlay reads the view model
//!
//! [`OverlayValues::value`] is handed the cell as the view model knows
//! it, and nothing else. An ordinary overlay therefore shows what the
//! character has perceived; it has no way to ask the world.

use serde::Deserialize;
use thiserror::Error;

use clash_world::Location;

use super::model::KnownCell;
use crate::color::{Color, ColorRef};
use crate::theme::Theme;

/// An `overlay` definition (12 §6.2).
///
/// ```json
/// { "type": "overlay", "id": "danger", "name": "Known hazards",
///   "cell": "known_field(it, 'fire') > 0", "shade": "red", "legend": "Fire" }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayDef {
    /// What the overlay is called where the player chooses one.
    #[serde(default)]
    pub name: Option<String>,
    /// An expression over a cell, as `it`: a condition, or a number from
    /// 0 to 1 for how strongly the cell is shaded. It is evaluated in an
    /// ordinary screen against what the party knows of the cell.
    pub cell: String,
    /// The colour cells are shaded with: a colour name or `#rrggbb`.
    pub shade: ColorRef,
    /// What the shading means, for a legend.
    #[serde(default)]
    pub legend: Option<String>,
}

/// Why an overlay cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OverlayError {
    /// The `cell` expression is empty.
    #[error("`cell` is empty: an overlay needs an expression over a cell")]
    NoCellExpression,
    /// A shade the theme does not know.
    #[error("unknown colour `{0}` in `shade`")]
    UnknownShade(String),
    /// `material`, `own` or `dress` as a shade.
    #[error("`{0}` in `shade` only means something in a glyph")]
    NotInAGlyph(&'static str),
}

impl OverlayError {
    /// Where in the definition the mistake is.
    pub fn path(&self) -> &'static str {
        match self {
            OverlayError::NoCellExpression => "cell",
            OverlayError::UnknownShade(_) | OverlayError::NotInAGlyph(_) => "shade",
        }
    }
}

/// An overlay ready to draw with: its shade resolved through a theme and
/// its expression kept as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    /// The definition's qualified id.
    pub id: String,
    /// What it is called.
    pub name: Option<String>,
    /// The source of the expression over a cell, not yet parsed.
    pub cell: String,
    /// The colour cells are shaded with.
    pub shade: Color,
    /// What the shading means.
    pub legend: Option<String>,
}

impl Overlay {
    /// Checks the definition and resolves its shade in `theme`. Every
    /// problem found is reported.
    pub fn compile(
        id: &str,
        def: &OverlayDef,
        theme: &Theme,
    ) -> Result<Overlay, Vec<OverlayError>> {
        let mut problems = Vec::new();
        if def.cell.trim().is_empty() {
            problems.push(OverlayError::NoCellExpression);
        }
        let shade = match &def.shade {
            ColorRef::Hex(rgb) => Some(Color::from_rgb(*rgb)),
            ColorRef::Named(name) => {
                let found = theme.color(name);
                if found.is_none() {
                    problems.push(OverlayError::UnknownShade(name.clone()));
                }
                found
            }
            ColorRef::Material => {
                problems.push(OverlayError::NotInAGlyph("material"));
                None
            }
            ColorRef::Own => {
                problems.push(OverlayError::NotInAGlyph("own"));
                None
            }
            ColorRef::Dress => {
                problems.push(OverlayError::NotInAGlyph("dress"));
                None
            }
        };
        match shade {
            Some(shade) if problems.is_empty() => Ok(Overlay {
                id: id.to_owned(),
                name: def.name.clone(),
                cell: def.cell.clone(),
                shade,
                legend: def.legend.clone(),
            }),
            _ => Err(problems),
        }
    }
}

/// The overlays the engine draws itself, named without a namespace
/// (12 §6.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BuiltinOverlay {
    /// The conduits of one resource, coloured by network. Not drawn yet:
    /// the view model holds no conduits.
    Networks,
    /// Areas by kind. Not drawn yet: the view model holds no areas.
    Areas,
    /// Planned work as ghosts. Not drawn yet: there are no jobs.
    Jobs,
    /// The temperature at each cell. Not drawn yet: the view model holds
    /// no temperatures.
    Temperature,
    /// The light at each cell, as shading, from the view model's light
    /// levels.
    Light,
}

impl BuiltinOverlay {
    /// All five, in the order of 12 §6.2.
    pub const ALL: [BuiltinOverlay; 5] = [
        BuiltinOverlay::Networks,
        BuiltinOverlay::Areas,
        BuiltinOverlay::Jobs,
        BuiltinOverlay::Temperature,
        BuiltinOverlay::Light,
    ];

    /// The name a `map_view` asks for it by.
    pub const fn name(self) -> &'static str {
        match self {
            BuiltinOverlay::Networks => "networks",
            BuiltinOverlay::Areas => "areas",
            BuiltinOverlay::Jobs => "jobs",
            BuiltinOverlay::Temperature => "temperature",
            BuiltinOverlay::Light => "light",
        }
    }

    /// The built-in overlay with a name, if it is one.
    pub fn from_name(name: &str) -> Option<BuiltinOverlay> {
        BuiltinOverlay::ALL
            .into_iter()
            .find(|overlay| overlay.name() == name)
    }
}

/// Evaluates an overlay's `cell` expression. This is where the binding
/// package (U6b) plugs in, as [`Evaluator`](crate::ui::Evaluator) does
/// for widget properties.
///
/// The cell is given as the view model holds it, so an overlay drawn
/// through this trait shows only what has been perceived.
pub trait OverlayValues: core::fmt::Debug {
    /// How strongly a known cell is shaded: `None` or zero for not at
    /// all, one or more for the overlay's full shade.
    fn value(&self, overlay: &Overlay, at: Location, cell: &KnownCell) -> Option<f64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_of_the_specification_compiles() {
        let def: OverlayDef = serde_json::from_str(
            r#"{ "name": "Known hazards", "cell": "known_field(it, 'fire') > 0",
                 "shade": "red", "legend": "Fire" }"#,
        )
        .unwrap();
        let overlay = Overlay::compile("demo:danger", &def, &Theme::default()).unwrap();
        assert_eq!(overlay.id, "demo:danger");
        assert_eq!(overlay.cell, "known_field(it, 'fire') > 0");
        assert_eq!(overlay.shade, Color::ansi(crate::Ansi16::Red));
        assert_eq!(overlay.legend.as_deref(), Some("Fire"));
    }

    #[test]
    fn every_problem_in_an_overlay_is_reported() {
        let def: OverlayDef =
            serde_json::from_str(r#"{ "cell": "  ", "shade": "octarine" }"#).unwrap();
        let problems = Overlay::compile("demo:bad", &def, &Theme::default()).unwrap_err();
        assert_eq!(
            problems,
            [
                OverlayError::NoCellExpression,
                OverlayError::UnknownShade("octarine".into())
            ]
        );
        assert_eq!(problems[1].path(), "shade");
        let def: OverlayDef =
            serde_json::from_str(r#"{ "cell": "true", "shade": "material" }"#).unwrap();
        assert_eq!(
            Overlay::compile("demo:bad", &def, &Theme::default()).unwrap_err(),
            [OverlayError::NotInAGlyph("material")]
        );
        assert!(serde_json::from_str::<OverlayDef>(r#"{ "shade": "red" }"#).is_err());
        assert!(
            serde_json::from_str::<OverlayDef>(r#"{ "cell": "1", "shade": "red", "x": 1 }"#)
                .is_err()
        );
    }

    #[test]
    fn built_in_overlays_are_known_by_name() {
        for overlay in BuiltinOverlay::ALL {
            assert_eq!(BuiltinOverlay::from_name(overlay.name()), Some(overlay));
        }
        assert_eq!(BuiltinOverlay::from_name("danger"), None);
    }
}
