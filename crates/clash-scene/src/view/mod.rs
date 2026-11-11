//! The view model and how the world is drawn (12 §5, §6).
//!
//! # The parts
//!
//! | Need | Use |
//! |------|-----|
//! | The scene's picture of the world | [`ViewModel`]: cells seen and remembered, sprites, heard markers |
//! | What an observer reports | [`CellContents`], [`EntitySprite`], [`HeardMarker`] |
//! | Truth, for debugging and tools | [`omniscient::fill`] |
//! | How each element state is drawn | [`WorldGlyphs`], built from the world's definitions and a theme |
//! | What one cell shows | [`Composer`]: the order of 12 §6.1, light, ambience, memory |
//! | What the view is centred on | [`Camera`] |
//! | All of it, drawn into a screen's views | [`WorldView`], which is the toolkit's [`ViewPainter`](crate::ui::ViewPainter) |
//! | How a condition looks | the `ambience` definition: [`AmbienceDef`], [`Ambience`] |
//! | Shading over a map | the `overlay` definition: [`OverlayDef`], [`Overlay`]; [`BuiltinOverlay`] |
//!
//! # How a screen's `map_view` comes to show the world
//!
//! ```no_run
//! use clash_scene::view::{WorldView, omniscient};
//! use clash_world::{CellBox, CellPos, Location};
//! # fn demo(ui: clash_scene::UiDefs, world: std::sync::Arc<clash_world::WorldDefs>,
//! #         cosmos: clash_world::Cosmos, grid: clash_world::GridId) -> Result<(), String> {
//! let mut scene = ui.scene("ui:default", "ui:default").map_err(|d| d.render())?;
//! let glyphs = ui.world_glyphs(&world, scene.theme()).map_err(|d| d.render())?;
//! let mut view = WorldView::new(glyphs, Location::new(grid, CellPos::new(8, 8, 0)));
//! // Perception and playback put what the party perceived into
//! // `view.model_mut()`. A debugging tool fills it from truth instead:
//! let cells = CellBox::new(CellPos::new(0, 0, -1), CellPos::new(31, 31, 2));
//! omniscient::fill(view.model_mut(), &cosmos, grid, cells).map_err(|e| e.to_string())?;
//! scene.set_world(view);
//! // Any screen with a `map_view` or a `section_view` node now draws it.
//! let screen = ui.screen("ui:worldview", scene.theme()).map_err(|d| d.render())?;
//! scene.open(screen);
//! # Ok(()) }
//! ```
//!
//! # Belief, not truth
//!
//! The view model holds what it was told and cannot read the world.
//! [`omniscient`] is the one module of this crate that can, and it is a
//! separate entry point that the application calls for a debugging view:
//! no screen, widget or binding leads to it.
//!
//! # What is not built
//!
//! - `region_map` and `chart_view` draw an empty area with their name
//!   ([`paint_region_map`], [`paint_chart_view`]).
//! - Subgrids are not composited and nothing is drawn through portals
//!   ([`SUBGRIDS_COMPOSITED`], [`PORTALS_DRAWN`]).
//! - Shapes of several cells, fields and decals: see [`WorldGlyphs`].
//! - An ambience's particles are read and kept, not drawn.
//! - Looks (12 §4.4) come with playback (package U5).

pub mod ambience;
pub mod camera;
pub mod compose;
pub mod glyphs;
pub mod model;
pub mod omniscient;
pub mod overlay;
pub mod paint;

pub use ambience::{Ambience, AmbienceDef, AmbienceError, ParticlesDef};
pub use camera::{Camera, MAX_ZOOM, SectionAxis};
pub use compose::{
    Composer, DECAL_TINT_MAX, DEPTH_KEEP, FIELD_TINT_MAX, LIGHT_FLOOR, LOOK_DOWN_DEFAULT,
    MEMORY_COLOR_KEEP, MEMORY_KEEP, OverlayDraw, Rank, Shown,
};
pub use glyphs::{
    COLOR_OVERLAY_LIGHT, FieldLook, GlyphProblem, MARK_BACKDROP, MARK_HEARD, MARK_PILE,
    MARK_SECTION_FLOOR, MARK_SUSPECTED, STYLE_CURSOR, ViewMarks, WorldGlyphs,
};
pub use model::{
    CellContents, DecalSeen, EntitySprite, FieldSeen, GridReader, HeardId, HeardMarker, KnownCell,
    LIGHT_FULL, SpriteId, SpriteKind, ViewModel,
};
pub use overlay::{BuiltinOverlay, Overlay, OverlayDef, OverlayError, OverlayValues};
pub use paint::{
    PORTALS_DRAWN, SUBGRIDS_COMPOSITED, WorldView, paint_chart_view, paint_region_map,
};
