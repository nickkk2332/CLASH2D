//! The window frontend on its own: a frame with a sprite gliding
//! between cells, a fading trail, a pulsing sprite at scale, and the
//! frame's effects in turn (none, shake, flash, zoom, bars, tint), with
//! the last input event and the cell under the pointer written out.
//!
//! ```text
//! cargo run -p clash-front-window --example effects
//! ```
//!
//! Escape or closing the window leaves.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clash_front_window::{WindowFrontend, WindowOptions};
use clash_scene::{
    Ansi16, Attrs, Borders, Canvas, CellStyle, Color, Frame, FrameCell, Frontend, InputEvent,
    MouseKind, Rect, Rgb, Sprite, Wash,
};

/// How long each effect is shown.
const EFFECT_TIME: f32 = 2.5;

/// How long a frame lasts.
const FRAME_TIME: Duration = Duration::from_millis(16);

/// The effects shown in turn, by name.
const EFFECTS: [&str; 6] = ["none", "shake", "flash", "zoom", "bars", "tint"];

fn main() -> ExitCode {
    let font = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/ui/assets/font.ttf");
    let options = WindowOptions {
        title: "CLASH2D: the window frontend".to_owned(),
        font,
        size: 16,
        backdrop: Rgb::new(0x10, 0x10, 0x10),
        grid: (64, 20),
    };
    let mut front = match WindowFrontend::start(options) {
        Ok(front) => front,
        Err(error) => {
            eprintln!("effects: {error}");
            return ExitCode::FAILURE;
        }
    };
    let began = Instant::now();
    let mut frame = Frame::new(0, 0);
    let mut said = String::from("nothing yet");
    let mut pointer = String::from("not over the grid");
    loop {
        for event in front.poll() {
            match event {
                InputEvent::Close => return ExitCode::SUCCESS,
                InputEvent::Key(key) if key.to_string() == "escape" => return ExitCode::SUCCESS,
                InputEvent::Key(key) => said = format!("key {key}"),
                InputEvent::Mouse(mouse) => {
                    pointer = format!("column {}, row {}", mouse.col, mouse.row);
                    if !matches!(mouse.kind, MouseKind::Move) {
                        said = format!("{:?} at {pointer}", mouse.kind);
                    }
                }
                other => said = format!("{other:?}"),
            }
        }
        let seconds = began.elapsed().as_secs_f32();
        build(&mut frame, &front, seconds, &said, &pointer);
        front.present(&frame);
        if let Some(problem) = front.take_error() {
            eprintln!("effects: {problem}");
            return ExitCode::FAILURE;
        }
        std::thread::sleep(FRAME_TIME);
    }
}

fn build(frame: &mut Frame, front: &WindowFrontend, seconds: f32, said: &str, pointer: &str) {
    let caps = front.caps();
    let text = CellStyle {
        fg: Color::new(Rgb::new(0xc8, 0xc8, 0xc8), Ansi16::White),
        bg: Color::new(Rgb::new(0x10, 0x10, 0x10), Ansi16::Black),
        attrs: Attrs::empty(),
    };
    let accent = CellStyle {
        fg: Color::new(Rgb::new(0xe0, 0xb0, 0x40), Ansi16::Yellow),
        attrs: Attrs::BOLD,
        ..text
    };
    frame.resize(caps.cols, caps.rows);
    frame.clear(FrameCell::new('.', text));
    let area = frame.area();
    let mut canvas = Canvas::new(frame);
    canvas.border(area, &Borders::default(), accent);
    canvas.print(
        2,
        1,
        "The window frontend. Escape or close to leave.",
        accent,
    );
    let effect = EFFECTS[(seconds / EFFECT_TIME) as usize % EFFECTS.len()];
    canvas.print(2, 2, &format!("Effect: {effect:<8}"), text);
    canvas.print(2, 3, &format!("Grid: {} by {}", caps.cols, caps.rows), text);
    canvas.print(2, 4, &format!("Last input: {said}"), text);
    canvas.print(2, 5, &format!("Pointer: {pointer}"), text);
    canvas.fill(Rect::new(2, 7, 20, 1), ' ', text);

    // A sprite gliding back and forth along row 7, with a fading trail.
    let span = 19.0;
    let along = (seconds * 4.0) % (2.0 * span);
    let x = 2.0
        + if along < span {
            along
        } else {
            2.0 * span - along
        };
    let red = Color::new(Rgb::new(0xff, 0x50, 0x40), Ansi16::BrightRed);
    for (n, alpha) in [(3.0, 0.15), (2.0, 0.3), (1.0, 0.5)] {
        let behind = if along < span { -n * 0.4 } else { n * 0.4 };
        frame.sprites.push(Sprite {
            alpha,
            ..Sprite::new('@', red, x + behind, 7.0)
        });
    }
    frame.sprites.push(Sprite {
        layer: 1,
        ..Sprite::new('@', red, x, 7.0)
    });
    let pulse = 1.5 + (seconds * 3.0).sin();
    frame.sprites.push(Sprite {
        scale: pulse,
        alpha: 0.8,
        ..Sprite::new(
            '*',
            Color::new(Rgb::new(0x60, 0xd0, 0xff), Ansi16::Cyan),
            30.0,
            9.0,
        )
    });

    let phase = (seconds % EFFECT_TIME) / EFFECT_TIME;
    match effect {
        "shake" => frame.fx.shake = 1.0 - phase,
        "flash" => {
            frame.fx.flash = Some(Wash {
                color: Rgb::WHITE,
                strength: 1.0 - phase,
            })
        }
        "zoom" => frame.fx.zoom = 1.0 + (phase * std::f32::consts::PI).sin(),
        "bars" => frame.fx.bars = 0.3 * (phase * std::f32::consts::PI).sin(),
        "tint" => {
            frame.fx.tint = Some(Wash {
                color: Rgb::new(0x20, 0x40, 0xa0),
                strength: 0.4,
            })
        }
        _ => {}
    }
}
