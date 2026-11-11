//! Golden frames (12 §12): a frame written as text and compared with a
//! stored file.
//!
//! [`frame_to_text`] writes a frame in a form that is the same on every
//! run and every machine. [`check_golden`] compares it with a file, and
//! with the environment variable `BLESS=1` writes the file instead, which
//! is how a golden frame is first made and how a deliberate change is
//! accepted:
//!
//! ```text
//! BLESS=1 cargo test -p clash-scene
//! ```
//!
//! # The text form
//!
//! ```text
//! clash-frame 1
//! size 12x2
//! chars
//! |┌──────────┐|
//! |│ Hello    │|
//! fg
//! |000000000000|
//! |001111100000|
//! bg
//! |222222222222|
//! |222222222222|
//! attrs
//! |............|
//! |..11111.....|
//! palette
//! 0 #707070 bright_black
//! 1 #e0b040 yellow
//! 2 #000000 black
//! sprites 0
//! fx shake=0.000 zoom=1.000 bars=0.000 flash=none tint=none
//! ```
//!
//! `chars` is the grid. `fg` and `bg` give each cell's colours as indices
//! into `palette`, which lists every colour in the frame with its
//! sixteen-colour fallback, in order of first use. An index is one
//! character from `0-9a-zA-Z`, or two when the frame has more than 62
//! colours. `attrs` gives each cell's attributes: `.` for none, otherwise
//! a base-32 digit of bold 1 + dim 2 + italic 4 + underline 8 +
//! reverse 16. Sprites follow, one per line, and the frame's effects.

use core::fmt::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::color::Color;
use crate::frame::Frame;

/// The environment variable that, set to `1`, makes [`check_golden`]
/// write the stored frame instead of comparing with it.
pub const BLESS_VAR: &str = "BLESS";

/// The digits palette indices are written in.
const INDEX_DIGITS: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Why a frame does not match its golden file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GoldenError {
    /// There is no stored frame yet.
    #[error(
        "no golden frame at {path}: run the test with BLESS=1 to write it, then check it by eye"
    )]
    Missing {
        /// The file that was looked for.
        path: PathBuf,
    },
    /// The frame differs from the stored one.
    #[error(
        "frame differs from {path} at line {line}\n  stored: {expected}\n  now:    {actual}\nrun with BLESS=1 to accept the change, and say why in the change's description"
    )]
    Differs {
        /// The stored file.
        path: PathBuf,
        /// The first line that differs, counted from 1.
        line: usize,
        /// That line in the stored file, or `<end of file>`.
        expected: String,
        /// That line of the frame as it is now, or `<end of file>`.
        actual: String,
    },
    /// The file could not be read or written.
    #[error("could not {action} {path}: {message}")]
    Io {
        /// `read` or `write`.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
}

fn write_index(out: &mut String, index: usize, width: usize) {
    let base = INDEX_DIGITS.len();
    let mut digits = [0u8; 4];
    let mut rest = index;
    for slot in digits.iter_mut().take(width) {
        *slot = INDEX_DIGITS[rest % base];
        rest /= base;
    }
    for digit in digits[..width].iter().rev() {
        out.push(char::from(*digit));
    }
}

/// Writes a frame as text. The same frame always gives the same text.
pub fn frame_to_text(frame: &Frame) -> String {
    let cols = usize::from(frame.cols);
    let rows = usize::from(frame.rows);
    let mut palette: Vec<Color> = Vec::new();
    let mut index_of = |color: Color| -> usize {
        match palette.iter().position(|c| *c == color) {
            Some(index) => index,
            None => {
                palette.push(color);
                palette.len() - 1
            }
        }
    };
    // Number the colours in a fixed order: cells row by row, foreground
    // before background, then sprites.
    let cell_colors: Vec<(usize, usize)> = frame
        .cells
        .iter()
        .map(|cell| (index_of(cell.fg), index_of(cell.bg)))
        .collect();
    let sprite_colors: Vec<usize> = frame.sprites.iter().map(|s| index_of(s.fg)).collect();
    let width = match palette.len() {
        0..=62 => 1,
        63..=3844 => 2,
        _ => 3,
    };

    let mut out = String::with_capacity(cols * rows * 5 + 256);
    let _ = writeln!(out, "clash-frame 1");
    let _ = writeln!(out, "size {}x{}", frame.cols, frame.rows);

    let row_cells = |row: usize| row * cols..(row + 1) * cols;
    out.push_str("chars\n");
    for row in 0..rows {
        out.push('|');
        for cell in &frame.cells[row_cells(row)] {
            out.push(if cell.ch.is_control() {
                '\u{fffd}'
            } else {
                cell.ch
            });
        }
        out.push_str("|\n");
    }
    for (name, pick) in [("fg", 0usize), ("bg", 1usize)] {
        out.push_str(name);
        out.push('\n');
        for row in 0..rows {
            out.push('|');
            for (fg, bg) in &cell_colors[row_cells(row)] {
                write_index(&mut out, if pick == 0 { *fg } else { *bg }, width);
            }
            out.push_str("|\n");
        }
    }
    out.push_str("attrs\n");
    for row in 0..rows {
        out.push('|');
        for cell in &frame.cells[row_cells(row)] {
            let bits = u32::from(cell.attrs.bits());
            out.push(if bits == 0 {
                '.'
            } else {
                char::from_digit(bits, 32).unwrap_or('?')
            });
        }
        out.push_str("|\n");
    }
    out.push_str("palette\n");
    for (index, color) in palette.iter().enumerate() {
        write_index(&mut out, index, width);
        let _ = writeln!(out, " {} {}", color.rgb, color.fallback);
    }
    let _ = writeln!(out, "sprites {}", frame.sprites.len());
    for (sprite, color) in frame.sprites.iter().zip(&sprite_colors) {
        let ch = if sprite.ch.is_control() {
            '\u{fffd}'
        } else {
            sprite.ch
        };
        let _ = write!(
            out,
            "{ch} x={:.3} y={:.3} scale={:.3} alpha={:.3} layer={} fg=",
            sprite.x, sprite.y, sprite.scale, sprite.alpha, sprite.layer
        );
        write_index(&mut out, *color, width);
        out.push('\n');
    }
    let wash = |wash: Option<crate::frame::Wash>| match wash {
        Some(w) => format!("{}@{:.3}", w.color, w.strength),
        None => "none".to_owned(),
    };
    let _ = writeln!(
        out,
        "fx shake={:.3} zoom={:.3} bars={:.3} flash={} tint={}",
        frame.fx.shake,
        frame.fx.zoom,
        frame.fx.bars,
        wash(frame.fx.flash),
        wash(frame.fx.tint)
    );
    out
}

/// Compares two frame texts and says where they first differ.
fn first_difference(expected: &str, actual: &str) -> Option<(usize, String, String)> {
    let mut stored = expected.lines();
    let mut now = actual.lines();
    let mut line = 0;
    loop {
        line += 1;
        match (stored.next(), now.next()) {
            (None, None) => return None,
            (a, b) if a == b => {}
            (a, b) => {
                let show = |l: Option<&str>| l.unwrap_or("<end of file>").to_owned();
                return Some((line, show(a), show(b)));
            }
        }
    }
}

/// Whether this run was asked to rewrite golden files.
pub fn blessing() -> bool {
    std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1")
}

/// Compares a frame with the one stored at `path`. With `BLESS=1` in the
/// environment the frame is written to `path` instead, creating
/// directories as needed, and the check passes.
pub fn check_golden(path: &Path, frame: &Frame) -> Result<(), GoldenError> {
    check_golden_text(path, &frame_to_text(frame), blessing())
}

/// The comparison behind [`check_golden`], with the text and the decision
/// to bless given by the caller.
pub fn check_golden_text(path: &Path, actual: &str, bless: bool) -> Result<(), GoldenError> {
    let io = |action: &'static str, error: std::io::Error| GoldenError::Io {
        action,
        path: path.to_owned(),
        message: error.to_string(),
    };
    if bless {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("write", e))?;
        }
        return std::fs::write(path, actual).map_err(|e| io("write", e));
    }
    let expected = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(GoldenError::Missing {
                path: path.to_owned(),
            });
        }
        Err(error) => return Err(io("read", error)),
    };
    match first_difference(&expected, actual) {
        None => Ok(()),
        Some((line, expected, actual)) => Err(GoldenError::Differs {
            path: path.to_owned(),
            line,
            expected,
            actual,
        }),
    }
}

/// [`check_golden`] for tests: panics with the difference.
///
/// # Panics
///
/// When the frame does not match the stored one, or there is none.
#[track_caller]
pub fn assert_golden(path: impl AsRef<Path>, frame: &Frame) {
    if let Err(error) = check_golden(path.as_ref(), frame) {
        panic!("{error}\n\nthe frame now:\n{}", frame.text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Ansi16, Rgb};
    use crate::frame::{Attrs, Canvas, CellStyle, FrameCell, Sprite, Wash};

    fn sample() -> Frame {
        let mut frame = Frame::new(6, 2);
        let base = CellStyle::default();
        let hot = CellStyle {
            fg: Color::new(Rgb::new(0xa0, 0x18, 0x28), Ansi16::Red),
            bg: base.bg,
            attrs: Attrs::BOLD | Attrs::UNDERLINE,
        };
        let mut canvas = Canvas::new(&mut frame);
        canvas.print(0, 0, "ab", base);
        canvas.print(2, 1, "cd", hot);
        frame.sprites.push(Sprite::new('*', hot.fg, 1.5, 0.25));
        frame.fx.flash = Some(Wash {
            color: Rgb::WHITE,
            strength: 0.5,
        });
        frame
    }

    const SAMPLE_TEXT: &str = "\
clash-frame 1
size 6x2
chars
|ab    |
|  cd  |
fg
|000000|
|002200|
bg
|111111|
|111111|
attrs
|......|
|..99..|
palette
0 #e5e5e5 white
1 #000000 black
2 #a01828 red
sprites 1
* x=1.500 y=0.250 scale=1.000 alpha=1.000 layer=0 fg=2
fx shake=0.000 zoom=1.000 bars=0.000 flash=#ffffff@0.500 tint=none
";

    #[test]
    fn a_frame_serialises_to_stable_text() {
        assert_eq!(frame_to_text(&sample()), SAMPLE_TEXT);
        assert_eq!(frame_to_text(&sample()), frame_to_text(&sample().clone()));
    }

    #[test]
    fn a_frame_compares_with_a_stored_one_and_bless_rewrites_it() {
        let dir = std::env::temp_dir().join(format!("clash-golden-{}", std::process::id()));
        let path = dir.join("nested").join("sample.frame");
        let _ = std::fs::remove_dir_all(&dir);

        let text = frame_to_text(&sample());
        assert_eq!(
            check_golden_text(&path, &text, false),
            Err(GoldenError::Missing { path: path.clone() })
        );
        check_golden_text(&path, &text, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE_TEXT);
        check_golden_text(&path, &text, false).unwrap();

        let mut changed = sample();
        changed.set(0, 1, FrameCell::new('x', CellStyle::default()));
        let error = check_golden_text(&path, &frame_to_text(&changed), false).unwrap_err();
        assert_eq!(
            error,
            GoldenError::Differs {
                path: path.clone(),
                line: 5,
                expected: "|  cd  |".into(),
                actual: "|x cd  |".into(),
            }
        );
        assert!(error.to_string().contains("BLESS=1"));

        check_golden_text(&path, &frame_to_text(&changed), true).unwrap();
        check_golden_text(&path, &frame_to_text(&changed), false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn many_colours_widen_the_indices() {
        let mut frame = Frame::new(70, 1);
        for (i, cell) in frame.cells.iter_mut().enumerate() {
            cell.fg = Color::from_rgb(Rgb::new(i as u8, 0, 0));
        }
        let text = frame_to_text(&frame);
        let fg_row = text.lines().nth(5).unwrap();
        assert_eq!(fg_row.chars().count(), 70 * 2 + 2);
        assert!(fg_row.starts_with("|00010203"));
        assert!(text.contains("\n17 #450000 black\n"));
    }

    #[test]
    fn a_shorter_or_longer_file_is_a_difference() {
        assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
        assert_eq!(
            first_difference("a\nb\n", "a\n"),
            Some((2, "b".into(), "<end of file>".into()))
        );
        assert_eq!(
            first_difference("a\n", "a\nc\n"),
            Some((2, "<end of file>".into(), "c".into()))
        );
    }
}
