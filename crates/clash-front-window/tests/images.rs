//! Frames drawn headless to images and compared with golden images
//! (package U3): one plain frame of text in colours and attributes, and
//! one for sprites and each of the frame's effects. Run with `BLESS=1`
//! to rewrite them, then look at them: any image viewer opens a PPM.

use std::path::PathBuf;

use clash_front_window::golden::assert_golden_image;
use clash_front_window::{GlyphAtlas, PixelBuffer, Renderer, theme_font};
use clash_scene::{Ansi16, Attrs, Canvas, CellStyle, Color, Frame, FrameCell, Rgb, Sprite, Wash};

/// The height of a cell in the images, in pixels: the bundled theme's.
const CELL_HEIGHT: u32 = 16;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.ppm"))
}

/// The bundled theme's font.
fn atlas() -> GlyphAtlas {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/ui/assets/font.ttf");
    GlyphAtlas::open(&path, CELL_HEIGHT).unwrap_or_else(|e| panic!("{e}"))
}

fn color(hex: &str, fallback: Ansi16) -> Color {
    Color::new(Rgb::parse(hex).unwrap(), fallback)
}

fn style(fg: Color, bg: Color, attrs: Attrs) -> CellStyle {
    CellStyle { fg, bg, attrs }
}

/// Ten cells by four: a border, words in each attribute, a character
/// the font lacks, and a control character.
fn sample() -> Frame {
    let ink = color("#101010", Ansi16::Black);
    let bone = color("#c8c8c8", Ansi16::White);
    let ash = color("#707070", Ansi16::BrightBlack);
    let amber = color("#e0b040", Ansi16::Yellow);
    let sky = color("#5090d0", Ansi16::BrightBlue);
    let crimson = color("#a01828", Ansi16::Red);
    let mut frame = Frame::new(10, 4);
    frame.clear(FrameCell::blank(style(bone, ink, Attrs::empty())));
    let mut canvas = Canvas::new(&mut frame);
    canvas.border(
        frame_area(),
        &clash_scene::Borders::default(),
        style(ash, ink, Attrs::empty()),
    );
    canvas.print(1, 1, "Bold", style(amber, ink, Attrs::BOLD));
    canvas.print(6, 1, "Ita", style(sky, ink, Attrs::ITALIC));
    canvas.print(1, 2, "un", style(bone, ink, Attrs::UNDERLINE));
    canvas.print(4, 2, "rv", style(crimson, ink, Attrs::REVERSE));
    canvas.print(7, 2, "dm", style(bone, ink, Attrs::DIM));
    canvas.print(
        2,
        3,
        "é\u{e000}\u{7}@",
        style(bone, crimson, Attrs::empty()),
    );
    frame
}

fn frame_area() -> clash_scene::Rect {
    clash_scene::Rect::new(0, 0, 10, 4)
}

/// The sample with sprites: one between cells, one larger and half
/// clear, one small, and two on the same spot on different layers.
fn with_sprites() -> Frame {
    let mut frame = sample();
    let green = color("#40e040", Ansi16::BrightGreen);
    let red = color("#ff3030", Ansi16::BrightRed);
    let white = color("#ffffff", Ansi16::BrightWhite);
    frame.sprites.extend([
        Sprite::new('@', green, 2.5, 1.25),
        Sprite {
            scale: 2.0,
            alpha: 0.6,
            ..Sprite::new('*', red, 6.0, 2.0)
        },
        Sprite {
            scale: 0.5,
            ..Sprite::new('o', white, 8.3, 0.6)
        },
        Sprite {
            layer: 1,
            ..Sprite::new('x', white, 4.75, 0.5)
        },
        Sprite::new('#', red, 4.75, 0.5),
    ]);
    frame
}

/// Draws a frame the size of its grid, with a dark backdrop where the
/// shake and the zoom uncover it.
fn picture(frame: &Frame) -> PixelBuffer {
    let mut atlas = atlas();
    let width = u32::from(frame.cols) * atlas.cell_width();
    let height = u32::from(frame.rows) * atlas.cell_height();
    let mut target = PixelBuffer::new(width, height);
    Renderer::new(Rgb::new(0x30, 0x20, 0x30)).draw(frame, &mut atlas, &mut target);
    target
}

#[test]
fn the_bundled_theme_draws_with_the_bundled_font_at_its_size() {
    let types = clash_scene::engine_types();
    let defs = clash_data::Loader::new(&types)
        .search_path(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods"))
        .load_mod_with_requirements("ui")
        .into_result()
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let font = theme_font(&defs, "ui:default").unwrap().unwrap();
    assert_eq!(u32::from(font.size), CELL_HEIGHT);
    let atlas = GlyphAtlas::open(&font.path, u32::from(font.size)).unwrap();
    assert_eq!((atlas.cell_width(), atlas.cell_height()), (8, 16));
}

/// Every character written anywhere in the repository's mods, glyphs,
/// screens, art and words alike, is in the bundled font, so that none
/// is drawn as the box of a missing character.
#[test]
fn the_bundled_font_has_every_character_the_bundled_mods_write() {
    let atlas = atlas();
    let mods = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods");
    let mut files = Vec::new();
    let mut dirs = vec![mods];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "json" || ext == "txt")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut missing = std::collections::BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for ch in text
            .chars()
            .filter(|ch| !ch.is_control() && !atlas.has_char(*ch))
        {
            missing
                .entry(ch)
                .or_insert_with(|| file.display().to_string());
        }
    }
    assert!(!files.is_empty());
    assert!(missing.is_empty(), "not in the font: {missing:?}");
}

#[test]
fn a_plain_frame_matches_its_image() {
    let image = picture(&sample());
    assert_eq!((image.width(), image.height()), (80, 64));
    assert_golden_image(golden("plain"), &image);
}

#[test]
fn sprites_match_their_image() {
    assert_golden_image(golden("sprites"), &picture(&with_sprites()));
}

#[test]
fn each_effect_matches_its_image() {
    let base = sample();
    let mut shaken = base.clone();
    shaken.fx.shake = 1.0;
    assert_golden_image(golden("shake"), &picture(&shaken));

    let mut flashed = base.clone();
    flashed.fx.flash = Some(Wash {
        color: Rgb::WHITE,
        strength: 0.5,
    });
    assert_golden_image(golden("flash"), &picture(&flashed));

    let mut tinted = base.clone();
    tinted.fx.tint = Some(Wash {
        color: Rgb::new(0, 40, 160),
        strength: 0.3,
    });
    assert_golden_image(golden("tint"), &picture(&tinted));

    let mut zoomed = with_sprites();
    zoomed.fx.zoom = 2.0;
    assert_golden_image(golden("zoom"), &picture(&zoomed));

    let mut barred = base.clone();
    barred.fx.bars = 0.25;
    assert_golden_image(golden("bars"), &picture(&barred));

    let mut all = with_sprites();
    all.fx.shake = 0.5;
    all.fx.zoom = 0.75;
    all.fx.bars = 0.5;
    all.fx.flash = Some(Wash {
        color: Rgb::new(255, 240, 200),
        strength: 0.25,
    });
    assert_golden_image(golden("everything"), &picture(&all));
}

#[test]
fn each_effect_changes_the_picture_and_only_effects_do() {
    let plain = picture(&sample());
    assert_eq!(picture(&sample()), plain, "drawing is the same every time");
    let effects: [fn(&mut Frame); 5] = [
        |f| f.fx.shake = 1.0,
        |f| {
            f.fx.flash = Some(Wash {
                color: Rgb::WHITE,
                strength: 0.5,
            })
        },
        |f| {
            f.fx.tint = Some(Wash {
                color: Rgb::new(0, 40, 160),
                strength: 0.3,
            })
        },
        |f| f.fx.zoom = 2.0,
        |f| f.fx.bars = 0.25,
    ];
    for (n, effect) in effects.iter().enumerate() {
        let mut frame = sample();
        effect(&mut frame);
        assert_ne!(picture(&frame), plain, "effect {n} shows");
    }
    // A bar of a quarter of the height in all: eight rows at each edge.
    let mut frame = sample();
    frame.fx.bars = 0.25;
    let barred = picture(&frame);
    for (y, black) in [
        (0, true),
        (7, true),
        (8, false),
        (55, false),
        (56, true),
        (63, true),
    ] {
        let row: Vec<_> = (0..80).map(|x| barred.get(x, y).unwrap()).collect();
        assert_eq!(row.iter().all(|&c| c == Rgb::BLACK), black, "row {y}");
    }
}
