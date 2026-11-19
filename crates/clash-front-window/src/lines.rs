//! Line-drawing characters and blocks drawn by the frontend instead of
//! taken from the font: [`draw_drawn`].
//!
//! A font's lines are anti-aliased where they fall between pixels, so a
//! border drawn from them is soft, and where the cell and the font's
//! line differ by a fraction of a pixel they may not meet. Drawn here
//! they are whole pixels, centred in the cell, and every arm reaches the
//! cell's edge, so a border is crisp and unbroken at any size, as
//! terminals draw them.
//!
//! Drawn: the light, heavy and double lines of U+2500 to U+257F that are
//! one weight throughout (straight lines, corners, tees and crosses),
//! and the full, upper, lower, left and right half blocks. The rest of
//! the range, mixed weights, dashes, arcs and diagonals, comes from the
//! font.

/// The weight of one arm of a line-drawing character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

use Weight::{Double as D, Heavy as H, Light as L, None as N};

/// One arm in each direction: up, right, down, left.
type Arms = [Weight; 4];

const UP: usize = 0;
const RIGHT: usize = 1;
const DOWN: usize = 2;
const LEFT: usize = 3;

/// A light line is one pixel thick for every this many pixels of cell
/// height, rounded, and at least one.
const LIGHT_HEIGHT_PER_PIXEL: u32 = 16;

/// The arms of the line-drawing characters drawn here.
fn arms(ch: char) -> Option<Arms> {
    Some(match ch {
        '─' => [N, L, N, L],
        '━' => [N, H, N, H],
        '│' => [L, N, L, N],
        '┃' => [H, N, H, N],
        '┌' => [N, L, L, N],
        '┏' => [N, H, H, N],
        '┐' => [N, N, L, L],
        '┓' => [N, N, H, H],
        '└' => [L, L, N, N],
        '┗' => [H, H, N, N],
        '┘' => [L, N, N, L],
        '┛' => [H, N, N, H],
        '├' => [L, L, L, N],
        '┣' => [H, H, H, N],
        '┤' => [L, N, L, L],
        '┫' => [H, N, H, H],
        '┬' => [N, L, L, L],
        '┳' => [N, H, H, H],
        '┴' => [L, L, N, L],
        '┻' => [H, H, N, H],
        '┼' => [L, L, L, L],
        '╋' => [H, H, H, H],
        '═' => [N, D, N, D],
        '║' => [D, N, D, N],
        '╔' => [N, D, D, N],
        '╗' => [N, N, D, D],
        '╚' => [D, D, N, N],
        '╝' => [D, N, N, D],
        '╠' => [D, D, D, N],
        '╣' => [D, N, D, D],
        '╦' => [N, D, D, D],
        '╩' => [D, D, N, D],
        '╬' => [D, D, D, D],
        '╴' => [N, N, N, L],
        '╵' => [L, N, N, N],
        '╶' => [N, L, N, N],
        '╷' => [N, N, L, N],
        '╸' => [N, N, N, H],
        '╹' => [H, N, N, N],
        '╺' => [N, H, N, N],
        '╻' => [N, N, H, N],
        _ => return None,
    })
}

/// A mask being drawn: its size and its coverage.
struct Canvas<'a> {
    width: i32,
    height: i32,
    mask: &'a mut [u8],
}

impl Canvas<'_> {
    /// Covers the pixels from `(x0, y0)` up to, not including, `(x1, y1)`.
    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (x0, x1) = (x0.clamp(0, self.width), x1.clamp(0, self.width));
        let (y0, y1) = (y0.clamp(0, self.height), y1.clamp(0, self.height));
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            self.mask[row + x0 as usize..row + x1 as usize].fill(u8::MAX);
        }
    }
}

/// Draws a line-drawing character or a block into a mask `width` by
/// `height`, if it is one drawn here; returns whether it was.
pub(crate) fn draw_drawn(ch: char, width: u32, height: u32, mask: &mut [u8]) -> bool {
    let mut canvas = Canvas {
        width: width as i32,
        height: height as i32,
        mask,
    };
    let (w, h) = (canvas.width, canvas.height);
    match ch {
        '█' => canvas.fill(0, 0, w, h),
        '▀' => canvas.fill(0, 0, w, h / 2),
        '▄' => canvas.fill(0, h / 2, w, h),
        '▌' => canvas.fill(0, 0, w / 2, h),
        '▐' => canvas.fill(w / 2, 0, w, h),
        _ => {
            let Some(arms) = arms(ch) else {
                return false;
            };
            draw_arms(&mut canvas, arms);
        }
    }
    true
}

/// Draws the arms of a line-drawing character from the cell's centre to
/// its edges.
fn draw_arms(canvas: &mut Canvas<'_>, arms: Arms) {
    let (w, h) = (canvas.width, canvas.height);
    let light = ((h as u32 + LIGHT_HEIGHT_PER_PIXEL / 2) / LIGHT_HEIGHT_PER_PIXEL).max(1) as i32;
    let thickness = |weight: Weight| match weight {
        Weight::Heavy => 2 * light + 1,
        _ => light,
    };
    // Where a line of a thickness runs: the first pixel across it.
    let across = |room: i32, thick: i32| (room - thick) / 2;

    // Single lines: each arm from the centre line's far side to the edge.
    for (arm, &weight) in arms.iter().enumerate() {
        if weight == Weight::None || weight == Weight::Double {
            continue;
        }
        let t = thickness(weight);
        // The widest line met at the centre, so that a corner is square.
        let meet = arms
            .iter()
            .filter(|&&other| other != Weight::None && other != Weight::Double)
            .map(|&other| thickness(other))
            .max()
            .unwrap_or(t);
        let (x, y) = (across(w, t), across(h, t));
        let (mx, my) = (across(w, meet), across(h, meet));
        match arm {
            UP => canvas.fill(x, 0, x + t, my + meet),
            DOWN => canvas.fill(x, my, x + t, h),
            LEFT => canvas.fill(0, y, mx + meet, y + t),
            _ => canvas.fill(mx, y, w, y + t),
        }
    }

    // Double lines: two rails to each arm, each ending where it meets a
    // rail of a neighbouring arm, or running on into the opposite arm.
    if !arms.contains(&Weight::Double) {
        return;
    }
    let t = light;
    let gap = (2 * light).max(2);
    let (cx, cy) = (across(w, t), across(h, t));
    let has = |arm: usize| arms[arm] == Weight::Double;
    // The two rails of each axis: before and after the centre line.
    let (top, bottom) = (cy - gap, cy + gap);
    let (left, right) = (cx - gap, cx + gap);
    if has(RIGHT) {
        let start = |side: usize, inner: i32, outer: i32| {
            if has(side) {
                inner
            } else if has(LEFT) {
                0
            } else {
                outer
            }
        };
        let x = start(UP, right, left);
        canvas.fill(x, top, w, top + t);
        let x = start(DOWN, right, left);
        canvas.fill(x, bottom, w, bottom + t);
    }
    if has(LEFT) {
        let end = |side: usize, inner: i32, outer: i32| {
            if has(side) {
                inner
            } else if has(RIGHT) {
                w
            } else {
                outer
            }
        };
        let x = end(UP, left + t, right + t);
        canvas.fill(0, top, x, top + t);
        let x = end(DOWN, left + t, right + t);
        canvas.fill(0, bottom, x, bottom + t);
    }
    if has(DOWN) {
        let start = |side: usize, inner: i32, outer: i32| {
            if has(side) {
                inner
            } else if has(UP) {
                0
            } else {
                outer
            }
        };
        let y = start(LEFT, bottom, top);
        canvas.fill(left, y, left + t, h);
        let y = start(RIGHT, bottom, top);
        canvas.fill(right, y, right + t, h);
    }
    if has(UP) {
        let end = |side: usize, inner: i32, outer: i32| {
            if has(side) {
                inner
            } else if has(DOWN) {
                h
            } else {
                outer
            }
        };
        let y = end(LEFT, top + t, bottom + t);
        canvas.fill(left, 0, left + t, y);
        let y = end(RIGHT, top + t, bottom + t);
        canvas.fill(right, 0, right + t, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(ch: char, width: u32, height: u32) -> Vec<String> {
        let mut mask = vec![0; (width * height) as usize];
        assert!(
            draw_drawn(ch, width, height, &mut mask),
            "{ch} is drawn here"
        );
        mask.chunks_exact(width as usize)
            .map(|row| row.iter().map(|&c| if c > 0 { '#' } else { '.' }).collect())
            .collect()
    }

    #[test]
    fn light_lines_are_one_pixel_and_reach_every_edge() {
        let line = drawn('─', 8, 16);
        assert_eq!(line[7], "########");
        assert_eq!(line.iter().filter(|row| row.contains('#')).count(), 1);
        let bar = drawn('│', 8, 16);
        assert!(bar.iter().all(|row| row == "...#...."));
        let corner = drawn('┌', 8, 16);
        assert_eq!(corner[6], "........");
        assert_eq!(corner[7], "...#####");
        assert_eq!(corner[15], "...#....");
        let cross = drawn('┼', 8, 16);
        assert_eq!(cross[0], "...#....");
        assert_eq!(cross[7], "########");
    }

    #[test]
    fn heavy_lines_are_thicker_and_meet_square() {
        let line = drawn('━', 8, 16);
        assert_eq!(line.iter().filter(|row| *row == "########").count(), 3);
        let corner = drawn('┏', 8, 16);
        assert_eq!(corner[6], "..######");
        assert_eq!(corner[8], "..######");
        assert_eq!(corner[9], "..###...");
    }

    #[test]
    fn double_lines_have_two_rails_joined_at_corners() {
        let line = drawn('═', 8, 16);
        assert_eq!(line[5], "########");
        assert_eq!(line[9], "########");
        assert_eq!(line.iter().filter(|row| row.contains('#')).count(), 2);
        let corner = drawn('╔', 8, 16);
        assert_eq!(corner[4], "........");
        assert_eq!(corner[5], ".#######");
        assert_eq!(corner[6], ".#......");
        assert_eq!(corner[9], ".#...###");
        assert_eq!(corner[10], ".#...#..");
        assert_eq!(corner[15], ".#...#..");
        let tee = drawn('╠', 8, 16);
        assert_eq!(tee[0], ".#...#..");
        assert_eq!(tee[5], ".#...###");
        assert_eq!(tee[7], ".#......");
        assert_eq!(tee[9], ".#...###");
        assert_eq!(tee[15], ".#...#..");
    }

    #[test]
    fn blocks_fill_their_part_of_the_cell() {
        assert!(drawn('█', 8, 16).iter().all(|row| row == "########"));
        let upper = drawn('▀', 8, 16);
        assert_eq!(
            (upper[7].as_str(), upper[8].as_str()),
            ("########", "........")
        );
        assert!(drawn('▐', 8, 16).iter().all(|row| row == "....####"));
        let mut mask = vec![0; 128];
        assert!(!draw_drawn('a', 8, 16, &mut mask));
        assert!(
            !draw_drawn('╭', 8, 16, &mut mask),
            "arcs come from the font"
        );
    }
}
