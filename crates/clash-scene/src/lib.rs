//! The CLASH2D presentation model: an event-sourced view of what the party
//! perceived, playback of simulation events over real time, the camera,
//! the interface toolkit and its state, and the cutscene director. It
//! turns simulation events and read-only views into frames of glyph cells
//! for a frontend, and nothing it computes flows back into the simulation.
//! Specified in `docs/12-presentation-and-ui.md`.
//!
//! # What is here
//!
//! | Need | Use |
//! |------|-----|
//! | What a frontend shows | [`Frame`], [`FrameCell`], [`Sprite`], [`FrameFx`]; draw through a [`Canvas`] |
//! | A colour | [`Color`]: an [`Rgb`] and its sixteen-colour fallback ([`Ansi16`]); the `color` definition [`ColorDef`] |
//! | Colour and style names | [`Theme`], built from a [`ThemeDef`] |
//! | A drawable thing's `glyph` | [`GlyphDef`], compiled to a [`Glyph`] |
//! | Text with `<name>…</>` tags | [`RichText`] |
//! | Input | [`InputEvent`], [`KeyChord`], the `keymap` definition [`KeymapDef`] |
//! | A window, a terminal or nothing | the [`Frontend`] trait; [`NullFrontend`] for tests |
//! | Golden frames | [`golden`] |
//! | Screens and widgets | [`ui`]: [`ui::ScreenDef`], [`ui::Screen`] |
//! | The scene itself | [`Scene`], over a [`SceneHost`] |
//! | Definitions from mods | [`register_defs`], then [`UiDefs`]; see [`defs`] |
//! | The picture of the world | [`view`]: [`ViewModel`], [`WorldView`], [`Camera`], [`WorldGlyphs`] |
//! | How a condition looks; shading over a map | the `ambience` and `overlay` definitions: [`AmbienceDef`], [`OverlayDef`] |
//!
//! # Outside the simulation
//!
//! This crate is not part of the simulation (01 §2): it may use ordinary
//! floating point, and nothing it computes is read by the simulation.
//! Frames must still be reproducible (12 invariant V8), so it keeps to
//! the workspace's determinism lints anyway: no hash-map iteration, no
//! wall clock, no platform-dependent float functions. Real time reaches
//! the scene only as a number handed to it by the application.
//!
//! # What is not here yet
//!
//! Playback, cutscenes and the link to the simulation come with packages
//! U5, U10 and U11 of `docs/plans/first-playable.md`. Until perception
//! and playback exist, nothing but a test or the omniscient fill of
//! [`view::omniscient`] puts anything into the view model. Bindings and
//! interface effects come with U6b; [`ui`] says where they plug in.

pub mod color;
pub mod defs;
pub mod frame;
pub mod frontend;
pub mod glyph;
pub mod golden;
pub mod input;
pub mod scene;
pub mod text;
pub mod theme;
pub mod ui;
pub mod view;

pub use color::{Ansi16, Color, ColorDef, ColorError, ColorRef, Rgb, ansi256};
pub use defs::{
    AmbienceId, ColorId, FigureId, KeymapId, OverlayId, ScreenId, ThemeId, UiDefs, check_defs,
    engine_types, register_defs, register_engine_defs,
};
pub use frame::{
    Attrs, Borders, Canvas, CellStyle, Frame, FrameCell, FrameFx, Rect, Sprite, Wash,
    is_single_width,
};
pub use frontend::{Caps, ColorDepth, Frontend, NullFrontend};
pub use glyph::{Facing, Glyph, GlyphDef, GlyphError, GlyphInputs, GlyphPart, GlyphPartDef, Paint};
pub use golden::{GoldenError, assert_golden, check_golden, frame_to_text};
pub use input::{
    CONTEXT_MENU, InputEvent, KeyChord, KeyCode, KeyError, KeymapDef, Mods, MouseButton,
    MouseEvent, MouseKind,
};
pub use scene::{NoHost, Scene, SceneHost, Turn};
pub use text::{RichText, TextError};
pub use theme::{BordersDef, FontDef, Style, StyleDef, Theme, ThemeDef, ThemeError};
pub use view::{
    Ambience, AmbienceDef, Camera, CellContents, EntitySprite, HeardMarker, KnownCell, Overlay,
    OverlayDef, SectionAxis, SpriteId, SpriteKind, ViewModel, WorldGlyphs, WorldView,
};
