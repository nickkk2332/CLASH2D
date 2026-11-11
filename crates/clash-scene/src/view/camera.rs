//! [`Camera`]: what a view of the world is centred on (12 §6.3).
//!
//! The camera follows a position, usually that of the active controlled
//! character. In look and targeting modes a free cursor takes its place,
//! and the view follows the cursor. The viewed storey is that of the
//! followed position, shifted up or down by the player. A view may be
//! zoomed out by whole factors.

use clash_world::{CellPos, GridId, Location};

/// The furthest a view can be zoomed out: this many cells along each
/// side of the block one glyph stands for.
pub const MAX_ZOOM: u32 = 16;

/// Along which line a section view cuts (12 §6.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SectionAxis {
    /// West to east through the camera's centre: columns run along `x`.
    #[default]
    EastWest,
    /// North to south through the camera's centre: columns run along
    /// `y`.
    NorthSouth,
}

impl SectionAxis {
    /// The other axis.
    pub const fn turned(self) -> SectionAxis {
        match self {
            SectionAxis::EastWest => SectionAxis::NorthSouth,
            SectionAxis::NorthSouth => SectionAxis::EastWest,
        }
    }
}

/// What a view of the world is centred on. See the [module](self).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Camera {
    grid: GridId,
    subject: CellPos,
    /// The free cursor's column and row, on the viewed storey.
    cursor: Option<(i32, i32)>,
    storey_shift: i32,
    zoom: u32,
    section: SectionAxis,
}

impl Camera {
    /// A camera following a position, at its storey, not zoomed out.
    pub const fn new(at: Location) -> Camera {
        Camera {
            grid: at.grid,
            subject: at.cell,
            cursor: None,
            storey_shift: 0,
            zoom: 1,
            section: SectionAxis::EastWest,
        }
    }

    /// Follows a position: called when the followed character moves. A
    /// move to another grid drops the cursor and the storey shift, which
    /// meant something only where they were set.
    pub fn follow(&mut self, at: Location) {
        if at.grid != self.grid {
            self.cursor = None;
            self.storey_shift = 0;
        }
        self.grid = at.grid;
        self.subject = at.cell;
    }

    /// The grid being viewed.
    pub const fn grid(&self) -> GridId {
        self.grid
    }

    /// The position being followed.
    pub const fn subject(&self) -> CellPos {
        self.subject
    }

    /// The storey being viewed: that of the followed position, shifted.
    pub const fn storey(&self) -> i32 {
        self.subject.z.saturating_add(self.storey_shift)
    }

    /// The free cursor, on the viewed storey, if there is one.
    pub const fn cursor(&self) -> Option<CellPos> {
        match self.cursor {
            Some((x, y)) => Some(CellPos::new(x, y, self.storey())),
            None => None,
        }
    }

    /// The cell at the middle of the view: the cursor, or else the
    /// followed position, on the viewed storey.
    pub const fn centre(&self) -> CellPos {
        match self.cursor {
            Some((x, y)) => CellPos::new(x, y, self.storey()),
            None => CellPos::new(self.subject.x, self.subject.y, self.storey()),
        }
    }

    /// Puts the free cursor on a column and row of the viewed storey.
    pub fn set_cursor(&mut self, x: i32, y: i32) {
        self.cursor = Some((x, y));
    }

    /// Moves the free cursor, starting it at the followed position if
    /// there is none.
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (x, y) = self.cursor.unwrap_or((self.subject.x, self.subject.y));
        self.cursor = Some((x.saturating_add(dx), y.saturating_add(dy)));
    }

    /// Drops the free cursor: the view follows its subject again.
    pub fn release_cursor(&mut self) {
        self.cursor = None;
    }

    /// Shifts the viewed storey up (positive) or down.
    pub fn shift_storey(&mut self, by: i32) {
        self.storey_shift = self.storey_shift.saturating_add(by);
    }

    /// How far the viewed storey is shifted from the followed position's.
    pub const fn storey_shift(&self) -> i32 {
        self.storey_shift
    }

    /// Views the followed position's own storey again.
    pub fn reset_storey(&mut self) {
        self.storey_shift = 0;
    }

    /// How far the view is zoomed out: the side, in cells, of the block
    /// one glyph stands for. 1 is one cell per glyph.
    pub const fn zoom(&self) -> u32 {
        self.zoom
    }

    /// Sets how far the view is zoomed out, from 1 to [`MAX_ZOOM`].
    pub fn set_zoom(&mut self, zoom: u32) {
        self.zoom = zoom.clamp(1, MAX_ZOOM);
    }

    /// Zooms out by one factor.
    pub fn zoom_out(&mut self) {
        self.set_zoom(self.zoom.saturating_add(1));
    }

    /// Zooms in by one factor.
    pub fn zoom_in(&mut self) {
        self.set_zoom(self.zoom.saturating_sub(1));
    }

    /// Along which line a section view cuts.
    pub const fn section_axis(&self) -> SectionAxis {
        self.section
    }

    /// Sets along which line a section view cuts.
    pub fn set_section_axis(&mut self, axis: SectionAxis) {
        self.section = axis;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(grid: u32, x: i32, y: i32, z: i32) -> Location {
        Location::new(GridId::new(grid), CellPos::new(x, y, z))
    }

    #[test]
    fn the_view_follows_its_subject_until_a_cursor_is_freed() {
        let mut camera = Camera::new(at(0, 10, 20, 2));
        assert_eq!(camera.centre(), CellPos::new(10, 20, 2));
        assert_eq!(camera.cursor(), None);
        camera.follow(at(0, 11, 20, 2));
        assert_eq!(camera.centre(), CellPos::new(11, 20, 2));

        camera.move_cursor(-3, 1);
        assert_eq!(camera.cursor(), Some(CellPos::new(8, 21, 2)));
        assert_eq!(camera.centre(), CellPos::new(8, 21, 2));
        // The subject moves on; the view stays with the cursor.
        camera.follow(at(0, 12, 20, 2));
        assert_eq!(camera.centre(), CellPos::new(8, 21, 2));
        camera.set_cursor(0, 0);
        assert_eq!(camera.centre(), CellPos::new(0, 0, 2));
        camera.release_cursor();
        assert_eq!(camera.centre(), CellPos::new(12, 20, 2));
    }

    #[test]
    fn the_storey_follows_the_subject_and_can_be_shifted() {
        let mut camera = Camera::new(at(0, 0, 0, 3));
        camera.shift_storey(-2);
        assert_eq!(camera.storey(), 1);
        camera.move_cursor(1, 0);
        assert_eq!(camera.cursor(), Some(CellPos::new(1, 0, 1)));
        // Climbing a stair keeps the shift.
        camera.follow(at(0, 0, 0, 4));
        assert_eq!(camera.storey(), 2);
        assert_eq!(camera.centre(), CellPos::new(1, 0, 2));
        camera.reset_storey();
        assert_eq!(camera.storey(), 4);
        // Another grid is another place: shift and cursor are dropped.
        camera.shift_storey(5);
        camera.follow(at(1, 7, 7, 0));
        assert_eq!(camera.grid(), GridId::new(1));
        assert_eq!(camera.centre(), CellPos::new(7, 7, 0));
        assert_eq!(camera.storey_shift(), 0);
    }

    #[test]
    fn zoom_stays_within_its_limits() {
        let mut camera = Camera::new(at(0, 0, 0, 0));
        camera.zoom_in();
        assert_eq!(camera.zoom(), 1);
        camera.zoom_out();
        camera.zoom_out();
        assert_eq!(camera.zoom(), 3);
        camera.set_zoom(1000);
        assert_eq!(camera.zoom(), MAX_ZOOM);
        camera.set_zoom(0);
        assert_eq!(camera.zoom(), 1);
        assert_eq!(camera.section_axis(), SectionAxis::EastWest);
        camera.set_section_axis(camera.section_axis().turned());
        assert_eq!(camera.section_axis(), SectionAxis::NorthSouth);
    }
}
