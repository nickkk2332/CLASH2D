//! The same frame in both frontends (package U3): the terminal's frame
//! text and the window's cells, as the renderer laid them out, hold the
//! same characters in the same cells.
//!
//! The frame is the `ui` mod's widget gallery as the terminal's own
//! tests build it, checked first against the terminal's stored golden
//! frame. The window draws it; then every cell of the picture is
//! compared with that cell's character, as the terminal shows it, drawn
//! alone in the cell's colours. A cell that held another character, or
//! a grid laid out a pixel off, would differ.

use std::path::PathBuf;

use clash_data::{Diagnostics, Loader};
use clash_front_window::{GlyphAtlas, Layout, PixelBuffer, Renderer, theme_font};
use clash_scene::{
    Ansi16, CellStyle, Color, Frame, FrameCell, NoHost, NullFrontend, Rgb, RichText, Scene, Sprite,
    UiDefs,
};

/// The fixed time step of scripted runs, in seconds.
const STEP: f64 = 1.0 / 60.0;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn describe(diagnostics: Diagnostics) -> String {
    diagnostics.render()
}

/// The `ui` mod, loaded.
fn ui_defs() -> UiDefs {
    let types = clash_scene::engine_types();
    let loaded = Loader::new(&types)
        .search_path(manifest().join("../../mods"))
        .load_mod_with_requirements("ui")
        .into_result()
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    UiDefs::from_defs(&loaded).unwrap_or_else(|e| panic!("{}", describe(e)))
}

/// The widget gallery's first frame at 80 by 24, built as the terminal
/// frontend's test builds it.
fn gallery_frame(defs: &UiDefs) -> Frame {
    let mut scene: Scene<NoHost> = defs
        .scene("ui:default", "ui:default")
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    let screen = defs
        .screen("ui:gallery", scene.theme())
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    scene.open(screen);
    for n in 1..=12 {
        let line = format!("Log line {n}.");
        scene.log_mut().push("demo", RichText::plain_text(&line));
    }
    let mut front = NullFrontend::new(80, 24);
    let mut frame = Frame::new(0, 0);
    scene.turn(&mut front, &mut frame, STEP);
    front.last().unwrap().clone()
}

/// The characters of a golden frame of the terminal frontend.
fn terminal_golden_chars(name: &str) -> Vec<Vec<char>> {
    let path = manifest()
        .join("../clash-front-term/tests/golden")
        .join(format!("{name}.frame"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .skip_while(|line| *line != "chars")
        .skip(1)
        .take_while(|line| *line != "fg")
        .map(|line| {
            let inner = line.strip_prefix('|').and_then(|l| l.strip_suffix('|'));
            inner
                .unwrap_or_else(|| panic!("a row of chars: {line:?}"))
                .chars()
                .collect()
        })
        .collect()
}

/// What the terminal shows of a frame: its grid with the sprites drawn
/// in their nearest cells, and a control character as a space.
fn terminal_view(frame: &Frame) -> Frame {
    let mut shown = frame.clone();
    shown.snap_sprites();
    for cell in &mut shown.cells {
        if cell.ch.is_control() {
            cell.ch = ' ';
        }
    }
    shown
}

/// The theme's font for the default theme.
fn atlas(defs: &UiDefs) -> GlyphAtlas {
    let font = theme_font(defs.defs(), "ui:default").unwrap().unwrap();
    GlyphAtlas::open(&font.path, u32::from(font.size)).unwrap_or_else(|e| panic!("{e}"))
}

/// Checks that every cell of the window's picture shows the character
/// the terminal shows in that cell, in the terminal's colours.
fn assert_same_cells(
    window: &PixelBuffer,
    layout: &Layout,
    terminal: &Frame,
    atlas: &mut GlyphAtlas,
) {
    assert_eq!((layout.cols, layout.rows), (terminal.cols, terminal.rows));
    let (width, height) = (layout.cell_width, layout.cell_height);
    let mut alone = PixelBuffer::new(width, height);
    let mut one = Frame::new(1, 1);
    let mut renderer = Renderer::default();
    let mut differing = Vec::new();
    for row in 0..terminal.rows {
        for col in 0..terminal.cols {
            let cell = *terminal.get(i32::from(col), i32::from(row)).unwrap();
            one.clear(cell);
            renderer.draw(&one, atlas, &mut alone);
            let (left, top, _, _) = layout.cell_rect(col, row);
            let same = (0..height).all(|y| {
                (0..width).all(|x| window.get(left as u32 + x, top as u32 + y) == alone.get(x, y))
            });
            if !same {
                differing.push((col, row, cell.ch));
            }
        }
    }
    assert!(
        differing.is_empty(),
        "cells the window draws other than the terminal shows: {differing:?}"
    );
}

#[test]
fn the_gallery_has_the_same_characters_in_the_same_cells_in_both_frontends() {
    let defs = ui_defs();
    let frame = gallery_frame(&defs);
    let terminal = terminal_view(&frame);
    let stored = terminal_golden_chars("gallery_lists");
    let shown: Vec<Vec<char>> = (0..terminal.rows)
        .map(|row| terminal.row_text(row).chars().collect())
        .collect();
    assert_eq!(
        shown, stored,
        "the frame is the one the terminal's golden frame stores"
    );

    let mut atlas = atlas(&defs);
    let missing: Vec<char> = terminal
        .cells
        .iter()
        .map(|cell| cell.ch)
        .filter(|&ch| ch != ' ' && !atlas.has_char(ch))
        .collect();
    assert!(missing.is_empty(), "the bundled font lacks {missing:?}");

    // A window a little larger than the grid: the grid is centred.
    let (width, height) = (80 * atlas.cell_width() + 5, 24 * atlas.cell_height() + 3);
    let mut window = PixelBuffer::new(width, height);
    Renderer::new(Rgb::new(1, 2, 3)).draw(&frame, &mut atlas, &mut window);
    let layout = Layout::new(frame.cols, frame.rows, &atlas, width, height);
    assert_eq!((layout.left, layout.top), (2, 1));
    assert_same_cells(&window, &layout, &terminal, &mut atlas);
}

/// One sprite to a cell, each over an empty cell: where sprites overlap
/// the window draws every one of them, the higher over the lower, and
/// the terminal only the highest; over a character the window shows
/// both and the terminal the sprite.
#[test]
fn sprites_on_whole_cells_land_in_the_cells_the_terminal_snaps_them_to() {
    let defs = ui_defs();
    let mut atlas = atlas(&defs);
    let ground = CellStyle {
        fg: Color::new(Rgb::new(0x80, 0x70, 0x50), Ansi16::Yellow),
        bg: Color::new(Rgb::new(0x10, 0x10, 0x18), Ansi16::Black),
        attrs: clash_scene::Attrs::empty(),
    };
    let mut frame = Frame::new(12, 5);
    frame.clear(FrameCell::new('.', ground));
    for (x, y) in [(2, 1), (5, 3), (9, 2)] {
        frame.set(x, y, FrameCell::blank(ground));
    }
    let red = Color::new(Rgb::new(0xe0, 0x30, 0x30), Ansi16::Red);
    frame.sprites.extend([
        Sprite::new('@', red, 2.0, 1.0),
        Sprite::new('d', red, 5.0, 3.0),
        Sprite {
            layer: 3,
            ..Sprite::new('!', red, 9.0, 2.0)
        },
    ]);
    let terminal = terminal_view(&frame);
    assert_eq!(terminal.row_text(2), ".........!..");
    let (width, height) = (12 * atlas.cell_width(), 5 * atlas.cell_height());
    let mut window = PixelBuffer::new(width, height);
    Renderer::default().draw(&frame, &mut atlas, &mut window);
    let layout = Layout::new(frame.cols, frame.rows, &atlas, width, height);
    assert_same_cells(&window, &layout, &terminal, &mut atlas);
}
