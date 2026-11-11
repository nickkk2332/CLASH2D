//! [`WorldView`]: the view model with its camera and glyph tables, and
//! the [`ViewPainter`] that draws it into the views of a screen
//! (12 §6.2).
//!
//! A screen definition places a `map_view` or a `section_view`; the
//! toolkit gives it a rectangle; and when the scene has a `WorldView`
//! ([`Scene::set_world`](crate::Scene::set_world)) that rectangle is
//! drawn here. Nothing in this module reads the world: it draws what the
//! view model holds.
//!
//! # The map view
//!
//! One storey from above. The camera's centre is the middle cell of the
//! rectangle (the cell left of and above the middle where the size is
//! even), east to the right and south down. Zoomed out by a factor `n`,
//! each glyph stands for a block of `n` by `n` cells and shows its most
//! significant cell ([`Rank`]); blocks are aligned to
//! multiples of `n`, so the picture does not shimmer as the cursor
//! moves. The factor is the camera's times the view's own `zoom`.
//!
//! # The section view
//!
//! A vertical slice along a line through the camera's centre: one
//! column per metre along the line, one row per storey, higher storeys
//! above, the camera's storey on the middle row. Along the camera's
//! [`SectionAxis`]: west to east with east to the right, or north to
//! south with south to the right. A floor is drawn as the theme's
//! `section_floor` mark, in the floor's own colours; a cell holding
//! nothing is air and blank. A section is not zoomed.
//!
//! # Not built
//!
//! `region_map` and `chart_view` need tracts and charts, which the view
//! model does not hold yet: [`paint_region_map`] and [`paint_chart_view`]
//! draw an empty area with the view's name. Subgrids are not composited
//! and nothing is drawn through portals: a view shows the cells of the
//! camera's grid alone ([`SUBGRIDS_COMPOSITED`], [`PORTALS_DRAWN`]).

use std::collections::BTreeMap;

use clash_world::{CellPos, Location};

use super::ambience::Ambience;
use super::camera::{Camera, MAX_ZOOM, SectionAxis};
use super::compose::{Composer, LOOK_DOWN_DEFAULT, OverlayDraw, Rank};
use super::glyphs::WorldGlyphs;
use super::model::ViewModel;
use super::overlay::{BuiltinOverlay, Overlay, OverlayValues};
use crate::frame::{Attrs, Canvas, FrameCell, Rect};
use crate::glyph::{GlyphInputs, resolve_part};
use crate::ui::{ViewDef, ViewKind, ViewPainter};

/// Whether a view composites subgrids over their parent's cells
/// (02 §18.1). Not yet: the world has no subgrids to composite.
pub const SUBGRIDS_COMPOSITED: bool = false;

/// Whether a view draws what is seen through a see-through portal beyond
/// it (12 §6.1). Not yet: the world has no portals.
pub const PORTALS_DRAWN: bool = false;

/// The scene's view of the world: the view model, the camera, and what
/// they are drawn with. See the [module](self).
#[derive(Debug)]
pub struct WorldView {
    model: ViewModel,
    camera: Camera,
    glyphs: WorldGlyphs,
    ambience: Option<Ambience>,
    overlays: BTreeMap<String, Overlay>,
    overlay_values: Option<Box<dyn OverlayValues>>,
    look_down: u8,
    /// Whether the line from the followed position to the cursor is
    /// drawn.
    line: bool,
}

/// The cells strictly between two cells of one storey, along the
/// straight line from one to the other.
fn line_between(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = (
        i64::from(to.0) - i64::from(from.0),
        i64::from(to.1) - i64::from(from.1),
    );
    let steps = dx.abs().max(dy.abs());
    // A line across more of the world than a view can show is not drawn.
    if !(2..=4096).contains(&steps) {
        return Vec::new();
    }
    (1..steps)
        .map(|step| {
            // Rounded to the nearest cell, halves away from the start.
            let along = |delta: i64| (2 * delta * step + steps * delta.signum()) / (2 * steps);
            (
                (i64::from(from.0) + along(dx)) as i32,
                (i64::from(from.1) + along(dy)) as i32,
            )
        })
        .collect()
}

impl WorldView {
    /// A view that knows nothing yet, drawn with these glyph tables and
    /// following this position.
    pub fn new(glyphs: WorldGlyphs, at: Location) -> WorldView {
        WorldView {
            model: ViewModel::new(),
            camera: Camera::new(at),
            glyphs,
            ambience: None,
            overlays: BTreeMap::new(),
            overlay_values: None,
            look_down: LOOK_DOWN_DEFAULT,
            line: false,
        }
    }

    /// Draws, or stops drawing, the line from the followed position to
    /// the free cursor in map views: what a picker shows while something
    /// is aimed (12 §9.3). It is drawn with the theme mark `line`.
    pub fn show_line(&mut self, on: bool) {
        self.line = on;
    }

    /// Whether the line to the cursor is drawn.
    pub fn line_shown(&self) -> bool {
        self.line
    }

    /// The cell a place in a map view stands for: `x` and `y` are a
    /// cell of the frame inside `area`, the view's rectangle, and `zoom`
    /// is the view's own. Zoomed out, it is the first cell of the block.
    /// `None` outside the view.
    pub fn map_cell_at(&self, zoom: u32, area: Rect, x: i32, y: i32) -> Option<CellPos> {
        if !area.contains(x, y) {
            return None;
        }
        let zoom = self.zoom_with(zoom);
        let centre = self.camera.centre();
        let (mid_col, mid_row) = ((area.w - 1) / 2, (area.h - 1) / 2);
        let first_x = (centre.x.div_euclid(zoom) - mid_col).wrapping_mul(zoom);
        let first_y = (centre.y.div_euclid(zoom) - mid_row).wrapping_mul(zoom);
        Some(CellPos::new(
            first_x.wrapping_add((x - area.x).wrapping_mul(zoom)),
            first_y.wrapping_add((y - area.y).wrapping_mul(zoom)),
            centre.z,
        ))
    }

    /// The view model.
    pub fn model(&self) -> &ViewModel {
        &self.model
    }

    /// The view model, to put what was perceived into it.
    pub fn model_mut(&mut self) -> &mut ViewModel {
        &mut self.model
    }

    /// The camera.
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// The camera, to move it.
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// The glyph tables the view is drawn with.
    pub fn glyphs(&self) -> &WorldGlyphs {
        &self.glyphs
    }

    /// Replaces the glyph tables: the theme or the definitions changed.
    pub fn set_glyphs(&mut self, glyphs: WorldGlyphs) {
        self.glyphs = glyphs;
    }

    /// The glyph tables, to give fields and decals their looks.
    pub fn glyphs_mut(&mut self) -> &mut WorldGlyphs {
        &mut self.glyphs
    }

    /// The ambience of where the viewed character is.
    pub fn ambience(&self) -> Option<&Ambience> {
        self.ambience.as_ref()
    }

    /// Sets or clears the ambience the view is drawn in.
    pub fn set_ambience(&mut self, ambience: Option<Ambience>) {
        self.ambience = ambience;
    }

    /// How many storeys a map view looks down through open cells: the
    /// setting `look_down`.
    pub fn look_down(&self) -> u8 {
        self.look_down
    }

    /// Sets how many storeys a map view looks down through open cells.
    pub fn set_look_down(&mut self, storeys: u8) {
        self.look_down = storeys;
    }

    /// Makes an overlay available to `map_view`s that name it by its
    /// qualified id.
    pub fn add_overlay(&mut self, overlay: Overlay) {
        self.overlays.insert(overlay.id.clone(), overlay);
    }

    /// The overlays available, in id order.
    pub fn overlays(&self) -> impl Iterator<Item = &Overlay> + '_ {
        self.overlays.values()
    }

    /// Sets what evaluates the `cell` expression of an `overlay`
    /// definition. Until one is set, such overlays draw nothing.
    pub fn set_overlay_values(&mut self, values: Option<Box<dyn OverlayValues>>) {
        self.overlay_values = values;
    }

    /// The overlay a view asks for, if it can be drawn.
    fn overlay_draw(&self, def: &ViewDef) -> OverlayDraw<'_> {
        let Some(name) = def.overlay.as_deref() else {
            return OverlayDraw::None;
        };
        if BuiltinOverlay::from_name(name) == Some(BuiltinOverlay::Light) {
            return OverlayDraw::Light;
        }
        match (self.overlays.get(name), &self.overlay_values) {
            (Some(overlay), Some(values)) => OverlayDraw::Custom(overlay, values.as_ref()),
            _ => OverlayDraw::None,
        }
    }

    fn composer(&self, def: &ViewDef) -> Composer<'_> {
        Composer {
            model: &self.model,
            glyphs: &self.glyphs,
            ambience: self.ambience.as_ref(),
            look_down: self.look_down,
            overlay: self.overlay_draw(def),
        }
    }

    /// How far a map view is zoomed out: the camera's factor times the
    /// view's own, within the limits.
    fn zoom_of(&self, def: &ViewDef) -> i32 {
        let own = def
            .zoom
            .as_static()
            .copied()
            .filter(|zoom| zoom.is_finite() && *zoom >= 1.0)
            .map_or(1, |zoom| zoom.round() as u32);
        self.zoom_with(own)
    }

    /// The camera's factor times a view's own, within the limits.
    fn zoom_with(&self, own: u32) -> i32 {
        self.camera
            .zoom()
            .saturating_mul(own.max(1))
            .clamp(1, MAX_ZOOM) as i32
    }

    /// Draws one storey from above into `area`.
    pub fn paint_map(&self, def: &ViewDef, area: Rect, canvas: &mut Canvas<'_>) {
        if area.is_empty() {
            return;
        }
        let composer = self.composer(def);
        let mut reader = self.model.reader(self.camera.grid());
        let centre = self.camera.centre();
        let zoom = self.zoom_of(def);
        // The block that holds the centre is drawn at the middle of the
        // area; blocks are aligned to multiples of the zoom.
        let (mid_col, mid_row) = ((area.w - 1) / 2, (area.h - 1) / 2);
        let first_x = (centre.x.div_euclid(zoom) - mid_col).wrapping_mul(zoom);
        let first_y = (centre.y.div_euclid(zoom) - mid_row).wrapping_mul(zoom);
        for row in 0..area.h {
            let y = first_y.wrapping_add(row.wrapping_mul(zoom));
            for col in 0..area.w {
                let x = first_x.wrapping_add(col.wrapping_mul(zoom));
                let cell = CellPos::new(x, y, centre.z);
                let shown = if zoom == 1 {
                    composer.plan_with(&mut reader, cell)
                } else {
                    composer.plan_block_with(&mut reader, cell, zoom as u32)
                };
                canvas.put(
                    area.x + col,
                    area.y + row,
                    shown.cell.ch,
                    shown.cell.style(),
                );
            }
        }
        let place = |cell: CellPos| -> Option<(i32, i32)> {
            let col = cell.x.wrapping_sub(first_x).div_euclid(zoom);
            let row = cell.y.wrapping_sub(first_y).div_euclid(zoom);
            (cell.z == centre.z && (0..area.w).contains(&col) && (0..area.h).contains(&row))
                .then_some((area.x + col, area.y + row))
        };
        self.paint_heard(canvas, &place);
        if let Some(cursor) = self.camera.cursor() {
            if self.line {
                // What stands in a cell is not drawn over: the line
                // shows through open ground only.
                let mut open =
                    |cell: CellPos| composer.plan_with(&mut reader, cell).rank <= Rank::Field;
                self.paint_line(canvas, cursor, &place, &mut open);
            }
            self.paint_cursor(canvas, place(cursor));
        }
    }

    /// Draws the line from the followed position to the cursor: the
    /// cells between the two that show nothing but ground, each keeping
    /// its background. A wall or a creature on the line stays as it is
    /// drawn. Only on the followed position's own storey, where the two
    /// are on one plane.
    fn paint_line(
        &self,
        canvas: &mut Canvas<'_>,
        cursor: CellPos,
        place: &dyn Fn(CellPos) -> Option<(i32, i32)>,
        open: &mut dyn FnMut(CellPos) -> bool,
    ) {
        let from = self.camera.subject();
        if from.z != cursor.z {
            return;
        }
        let mark = self.glyphs.marks().line;
        let base = self.glyphs.base();
        for (x, y) in line_between((from.x, from.y), (cursor.x, cursor.y)) {
            let cell = CellPos::new(x, y, cursor.z);
            let Some((col, row)) = place(cell) else {
                continue;
            };
            if !open(cell) {
                continue;
            }
            canvas.edit(col, row, |cell| {
                let beneath = crate::CellStyle {
                    bg: cell.bg,
                    ..base
                };
                *cell = resolve_part(mark, &GlyphInputs::default(), beneath);
            });
        }
    }

    /// Draws a vertical slice along the camera's section axis into
    /// `area`.
    pub fn paint_section(&self, def: &ViewDef, area: Rect, canvas: &mut Canvas<'_>) {
        if area.is_empty() {
            return;
        }
        let composer = self.composer(def);
        let mut reader = self.model.reader(self.camera.grid());
        let centre = self.camera.centre();
        let axis = self.camera.section_axis();
        let (mid_col, mid_row) = ((area.w - 1) / 2, (area.h - 1) / 2);
        let along = |col: i32| -> (i32, i32) {
            let step = col - mid_col;
            match axis {
                SectionAxis::EastWest => (centre.x.wrapping_add(step), centre.y),
                SectionAxis::NorthSouth => (centre.x, centre.y.wrapping_add(step)),
            }
        };
        for row in 0..area.h {
            let z = centre.z.saturating_add(mid_row - row);
            for col in 0..area.w {
                let (x, y) = along(col);
                let shown = composer.section_with(&mut reader, CellPos::new(x, y, z));
                canvas.put(
                    area.x + col,
                    area.y + row,
                    shown.cell.ch,
                    shown.cell.style(),
                );
            }
        }
        let place = |cell: CellPos| -> Option<(i32, i32)> {
            let (on_line, step) = match axis {
                SectionAxis::EastWest => (cell.y == centre.y, cell.x.wrapping_sub(centre.x)),
                SectionAxis::NorthSouth => (cell.x == centre.x, cell.y.wrapping_sub(centre.y)),
            };
            let col = mid_col.wrapping_add(step);
            let row = mid_row.wrapping_sub(cell.z.wrapping_sub(centre.z));
            (on_line && (0..area.w).contains(&col) && (0..area.h).contains(&row))
                .then_some((area.x + col, area.y + row))
        };
        self.paint_heard(canvas, &place);
        if let Some(cursor) = self.camera.cursor() {
            self.paint_cursor(canvas, place(cursor));
        }
    }

    /// Draws the markers for what was only heard, where they fall in the
    /// view. A marker keeps the background of the cell it is drawn on.
    fn paint_heard(&self, canvas: &mut Canvas<'_>, place: &dyn Fn(CellPos) -> Option<(i32, i32)>) {
        let grid = self.camera.grid();
        let mark = self.glyphs.marks().heard;
        for marker in self.model.heard() {
            if marker.loc.grid != grid {
                continue;
            }
            let Some((x, y)) = place(marker.loc.cell) else {
                continue;
            };
            let mut part = mark;
            if marker.ch.is_some() {
                part.ch = marker.ch;
            }
            let base = self.glyphs.base();
            canvas.edit(x, y, |cell| {
                let beneath = crate::CellStyle {
                    bg: cell.bg,
                    ..base
                };
                *cell = resolve_part(part, &GlyphInputs::default(), beneath);
            });
        }
    }

    /// Highlights the cell under the cursor.
    fn paint_cursor(&self, canvas: &mut Canvas<'_>, at: Option<(i32, i32)>) {
        let Some((x, y)) = at else {
            return;
        };
        let style = self.glyphs.cursor();
        canvas.edit(x, y, |cell| {
            *cell = FrameCell::new(cell.ch, style.over(cell.style()));
        });
    }
}

/// Draws an empty area with a name at its middle: what a view that is
/// not built yet shows.
fn paint_unbuilt(glyphs: &WorldGlyphs, name: &str, area: Rect, canvas: &mut Canvas<'_>) {
    let base = glyphs.base();
    canvas.fill(area, ' ', base);
    let len = name.chars().count() as i32;
    if area.h > 0 && area.w >= len {
        let style = crate::CellStyle {
            attrs: base.attrs | Attrs::DIM,
            ..base
        };
        canvas.print(
            area.x + (area.w - len) / 2,
            area.y + area.h / 2,
            name,
            style,
        );
    }
}

/// Draws a `region_map` (12 §6.2): the tract-scale map.
///
/// A stub: the view model does not hold tracts, their glyphs,
/// devastation or known sites yet (world-model packages W8 to W10). It
/// draws an empty area with the view's name.
pub fn paint_region_map(view: &WorldView, _def: &ViewDef, area: Rect, canvas: &mut Canvas<'_>) {
    paint_unbuilt(&view.glyphs, ViewKind::Region.name(), area, canvas);
}

/// Draws a `chart_view` (12 §6.2): a chart and the travelers on it.
///
/// A stub: charts are world-model package W21, and the view model holds
/// no travelers. It draws an empty area with the view's name.
pub fn paint_chart_view(view: &WorldView, _def: &ViewDef, area: Rect, canvas: &mut Canvas<'_>) {
    paint_unbuilt(&view.glyphs, ViewKind::Chart.name(), area, canvas);
}

impl ViewPainter for WorldView {
    fn paint(
        &self,
        kind: ViewKind,
        def: &ViewDef,
        _focused: bool,
        area: Rect,
        canvas: &mut Canvas<'_>,
    ) {
        match kind {
            ViewKind::Map => self.paint_map(def, area, canvas),
            ViewKind::Section => self.paint_section(def, area, canvas),
            ViewKind::Region => paint_region_map(self, def, area, canvas),
            ViewKind::Chart => paint_chart_view(self, def, area, canvas),
        }
    }
}
